use std::{fs::{File, create_dir_all, remove_file}, io::{Seek, SeekFrom, copy}, path::PathBuf};
use librespot::{audio::{AudioDecrypt, AudioFile}, core::{Error, FileId, SpotifyId, SpotifyUri}, metadata::{Artist, Track, album::AlbumType, audio::{AudioFileFormat, AudioFiles}}};
use log::{error, info, warn, debug};
use sanitize_filename::sanitize;
use lofty::{config::WriteOptions, picture::{Picture, PictureType}, tag::{ItemKey, ItemValue, Tag, TagExt, TagItem, TagType}};
use crate::{config::{FORMAT_PREFERENCE, SPOTIFY_OGG_HEADER_END, format_data_rate, format_tag_type, get_extension_from_format}, cover::get_cover, ctx::DLContext};

pub fn artists_string(track: &Track) -> String {
    track.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(" & ")
}

pub fn get_artist_genre(ctx: &DLContext, artist: Option<&Artist>) -> String {
    ctx.genre.clone().unwrap_or_else(|| {
        if let Some(b62id) = artist.and_then(|a| a.id.to_id().ok()) {
            ctx.config.artist_genres
                .get(&b62id)
                .and_then(|v| v.get(0))
                .unwrap_or(&ctx.config.default_genre)
                .clone()
        } else {
            ctx.config.default_genre.clone()
        }
    })
}

pub struct TrackFileDescriptor {
    pub base: String,
    pub stem: String,
    pub extension: String,
    pub name: String,
    pub path: PathBuf
}

impl TrackFileDescriptor {
    pub fn new(ctx: &DLContext, track: &Track, format: AudioFileFormat) -> Option<Self> {
        let b62id = track.id.to_id().ok()?;
        let basepath = Self::get_track_path(ctx, &track);
        let base = format!("{} - {} ", artists_string(&track), track.name);
        let stem = format!("{}({})", base, b62id);
        let extension = get_extension_from_format(format).to_string();
        let name = format!("{}.{}", stem, extension);
        let name = sanitize(name);
        let name = name.chars().take(200).collect::<String>();
        let path = basepath.join(&name);
        Some(Self { base, stem, extension, name, path })
    }

    pub fn get_track_path(ctx: &DLContext, track: &Track) -> PathBuf {
        let mut dirpath = PathBuf::from(ctx.config.root_folder.clone());
        if let Some(artist) = track.album.artists.0.get(0) {
            dirpath.push(get_artist_genre(ctx, Some(artist)));
            dirpath.push(artist.name.clone());
        }
        if track.album.album_type == AlbumType::SINGLE {
            dirpath.push(ctx.config.singles_folder.clone());
        } else {
            dirpath.push(&track.album.name);
        }
        dirpath
    }
}

pub struct AudioFileFormatExt {
    pub aff: AudioFileFormat,
    pub fid: FileId,
    pub tgt: TagType
}

impl AudioFileFormatExt {
    pub fn new(track: &Track) -> Option<Self> {
        let (format, file_id) = Self::select_best_format(track).ok().map(|(f, id)| (f, *id))?;
        let tag_type = format_tag_type(format);
        Some(Self { aff: format, fid: file_id, tgt: tag_type })
    }

    pub fn select_best_format(track: &Track) -> Result<(AudioFileFormat, &FileId), Error> {
        let b62id = track.id.to_id()?;
        let fids = track.files
            .keys()
            .map(|f| format!("{:?}", f))
            .collect::<Vec<_>>()
            .join(", ");
        debug!("<{}-{}> Track Format/Available: {}", b62id, track.number, fids);
        let (format, file_id) = FORMAT_PREFERENCE.iter()
            .find_map(|&format| track.files.get(&format).map(|id| (format, id)))
            .ok_or_else(|| Error::failed_precondition("No format available"))?;
        debug!("<{}-{}> Track Format/Selected: {:?}", b62id, track.number, format);
        Ok((format, file_id))
    }
}

pub struct TrackExt<'a> {
    ctx: &'a mut DLContext,
    pub id: SpotifyId,
    pub b62id: String,
    pub inner: Track,
    pub format: AudioFileFormatExt,
    pub file: TrackFileDescriptor,
}

impl<'a> TrackExt<'a> {
    pub fn new(ctx: &'a mut DLContext, inner: Track) -> Option<Self> {
        let (id, b62id) = Self::id(&inner)?;
        let format = AudioFileFormatExt::new(&inner)?;
        let file = TrackFileDescriptor::new(ctx, &inner, format.aff)?;
        let track = Self { ctx, id, b62id, inner, format, file };
        Some(track)
    }

    pub fn id(track: &Track) -> Option<(SpotifyId, String)> {
        match track.id {
            SpotifyUri::Track { id } => Some((id, id.to_base62().ok()?)),
            _ => return None,
        }
    }

    pub async fn get_audio_file(&self) -> Result<AudioDecrypt<AudioFile>, Error> {
        let bytes_per_second = format_data_rate(self.format.aff);
        let encrypted_file = AudioFile::open(&self.ctx.session, self.format.fid, bytes_per_second).await?;
        let key = self.ctx.session.audio_key().request(self.id, self.format.fid).await?;
        let mut decrypted_file = AudioDecrypt::new(Some(key), encrypted_file);
        let offset = if AudioFiles::is_ogg_vorbis(self.format.aff) { SPOTIFY_OGG_HEADER_END } else { 0 };
        decrypted_file.seek(SeekFrom::Start(offset))?;
        Ok(decrypted_file)
    }

    pub async fn save_audio(&self) -> Result<(), Error> {
        let mut decrypted_file = self.get_audio_file().await?;
        create_dir_all(self.file.path.parent().unwrap())?;
        let mut outfile = File::create(&self.file.path)?;
        copy(&mut decrypted_file, &mut outfile)?;
        info!("<{}-{}> Track Save \"{:?}\"", self.b62id, self.inner.number, self.file.path);
        Ok(())
    }

    pub async fn deduplication_check(&self) -> Result<(), Error> {
        if let Some(occurrences) = self.ctx.occurences.get(&self.file.base) {
            let len = occurrences.len();
            if len > 2 {
                warn!("<{}-{}> Track Skip/Multiple: \"{}\"", self.b62id, self.inner.number, self.inner.name);
                return Err(Error::already_exists(""));
            }
            for path in occurrences {
                let Some(path_str) = path.to_str() else {
                    continue;
                };
                let occurrence_single = path_str.contains(&self.ctx.config.singles_folder);
                let current_single = self.inner.album.album_type == AlbumType::SINGLE;
                if occurrence_single == current_single {
                    warn!("<{}-{} > Track Skip/Exists \"{}\"", self.b62id, self.inner.number, self.inner.name);
                    return Err(Error::already_exists(""));
                }
                if occurrence_single {
                    return match remove_file(path) {
                        Ok(_) => {
                            warn!("<{}-{}> Track Remove/Duplicate \"{}\"", self.b62id, self.inner.number, path_str);
                            Ok(())
                        }
                        Err(_) => {
                            error!("<{}-{}> Track Fail/Remove \"{}\"", self.b62id, self.inner.number, path_str);
                            Err(Error::unavailable(""))
                        }
                    };
                }
            }
            if len > 1 {
                return Err(Error::already_exists(""));
            }
        }
        Ok(())
    }

    pub async fn apply_tag(&mut self) -> Result<(), Error> {
        let mut tag = Tag::new(self.format.tgt);
        tag.insert(TagItem::new(ItemKey::TrackTitle, ItemValue::Text(self.inner.name.clone())));
        tag.insert(TagItem::new(ItemKey::AlbumTitle, ItemValue::Text(self.inner.album.name.clone())));
        tag.insert(TagItem::new(ItemKey::TrackArtist, ItemValue::Text(artists_string(&self.inner))));
        tag.insert(TagItem::new(ItemKey::TrackNumber, ItemValue::Text(self.inner.number.to_string())));
        tag.insert(TagItem::new(ItemKey::Isrc, ItemValue::Text(self.inner.id.to_uri()?)));

        let cover = get_cover(self.ctx, &self.inner).await?;
        let picture = Picture::new_unchecked(PictureType::CoverFront, Some(cover.mime.clone()), Some("cover".to_string()), cover.data.clone());
        tag.push_picture(picture);

        if let Err(e) = tag.save_to_path(&self.file.path, WriteOptions::default()) {
            warn!("<{}-{}> Track Metadata/Fail \"{:?}\": {}", self.b62id, self.inner.number, self.file.path, e);
        } else {
            debug!("<{}-{}> Metadata written to {:?}", self.b62id, self.inner.number, self.file.path);
        }

        Ok(())
    }

    pub async fn download(&mut self) -> Result<(), Error> {
        self.deduplication_check().await?;
        self.save_audio().await?;
        self.apply_tag().await?;
        Ok(())
    }
}
