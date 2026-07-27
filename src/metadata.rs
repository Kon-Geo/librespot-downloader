use std::{fs::{File, create_dir_all, remove_file}, io::{Seek, SeekFrom, copy}, path::PathBuf};
use librespot::{audio::{AudioDecrypt, AudioFile}, core::{Error, FileId, SpotifyId, SpotifyUri}, metadata::{Artist, Track, album::AlbumType, audio::{AudioFileFormat, AudioFiles}}};
use log::{error, info, warn, debug};
use sanitize_filename::sanitize;
use lofty::{config::WriteOptions, picture::{Picture, PictureType}, tag::{ItemKey, ItemValue, Tag, TagExt, TagItem, TagType}};
use crate::{config::{FORMAT_PREFERENCE, SPOTIFY_OGG_HEADER_END, format_data_rate, format_tag_type, get_extension_from_format}, ctx::DLContext};

pub fn artists_string(track: &Track) -> String {
    track.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(" & ")
}

pub fn get_artist_genre(ctx: &DLContext, artist: Option<&Artist>) -> String {
    if let Some(b62id) = artist.and_then(|a| a.id.to_id().ok()) {
        ArtistExt::get_genre(ctx, &b62id)
    } else {
        ctx.config.default_genre.clone()
    }
}

pub struct ArtistExt {
    pub id: SpotifyId,
    pub b62id: String,
    pub inner: Artist,
    pub folder: PathBuf,
    pub genre: String,
}

impl ArtistExt {
    pub fn new(ctx: &DLContext, inner: Artist) -> Result<Self, Error> {
        let (id, b62id) = Self::id(&inner)?;
        let genre = Self::get_genre(ctx, &b62id);
        let folder = Self::get_path(ctx, &inner, &genre);
        let artist = Self { id, b62id, inner, genre, folder };
        Ok(artist)
    }

    pub fn id(artist: &Artist) -> Result<(SpotifyId, String), Error> {
        match artist.id {
            SpotifyUri::Artist { id } => Ok((id, id.to_base62()?)),
            _ => return Err(Error::failed_precondition("Invalid Artist ID")),
        }
    }

    pub fn get_genre(ctx: &DLContext, b62id: &String) -> String {
        ctx.config.artist_genres
            .get(b62id)
            .and_then(|v| v.get(0))
            .unwrap_or(&ctx.config.default_genre)
            .clone()
    }

    pub fn get_path(ctx: &DLContext, artist: &Artist, genre: &String) -> PathBuf {
        let mut dirpath = PathBuf::from(ctx.config.root_folder.clone());
        dirpath.push(genre);
        dirpath.push(artist.name.clone());
        dirpath
    }

    pub fn track(&self, ctx: &mut DLContext) {
        ctx.filedb.track_artist(&self.b62id, &self.folder);
        if !ctx.config.artist_genres.contains_key(&self.b62id) {
            info!("<{}> Artist Track \"{}\"", self.id, self.inner.name);
        }
    }
}

pub struct TrackFileDescriptor {
    pub base: String,
    pub stem: String,
    pub extension: String,
    pub name: String,
    pub path: PathBuf
}

impl TrackFileDescriptor {
    pub fn new(ctx: &DLContext, track: &Track, format: AudioFileFormat) -> Result<Self, Error> {
        let b62id = track.id.to_id()?;
        let basepath = Self::get_path(ctx, &track);
        let base = format!("{} - {} ", artists_string(&track), track.name);
        let stem = format!("{}({})", base, b62id);
        let extension = get_extension_from_format(format).to_string();
        let name = format!("{}.{}", stem, extension);
        let name = sanitize(name);
        let name = name.chars().take(200).collect::<String>();
        let path = basepath.join(&name);
        let file = Self { base, stem, extension, name, path };
        Ok(file)
    }

    pub fn get_path(ctx: &DLContext, track: &Track) -> PathBuf {
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
    pub format: AudioFileFormat,
    pub file_id: FileId,
    pub tag_type: TagType,
}

impl AudioFileFormatExt {
    pub fn new(track: &Track) -> Result<Self, Error> {
        let (format, file_id) = Self::select_best_format(&track).map(|(f, id)| (f, *id))?;
        let tag_type = format_tag_type(format);
        let affext = Self { format, file_id, tag_type };
        Ok(affext)
    }

    pub fn select_best_format(track: &Track) -> Result<(AudioFileFormat, &FileId), Error> {
        let b62id = track.id.to_id()?;
        let fids = track.files
            .keys()
            .map(|f| format!("{:?}", f))
            .collect::<Vec<_>>()
            .join(", ");
        debug!("<{}-{}> Format Available: {}", b62id, track.number, fids);
        let (format, file_id) = FORMAT_PREFERENCE.iter()
            .find_map(|&format| track.files.get(&format).map(|id| (format, id)))
            .ok_or_else(|| Error::failed_precondition("No format available"))?;
        debug!("<{}-{}> Format Selected: {:?}", b62id, track.number, format);
        Ok((format, file_id))
    }

    pub async fn get_audio_file(&self, ctx: &DLContext, id: SpotifyId) -> Result<AudioDecrypt<AudioFile>, Error> {
        let bytes_per_second = format_data_rate(self.format);
        let encrypted_file = AudioFile::open(&ctx.session, self.file_id, bytes_per_second).await?;
        let key = ctx.session.audio_key().request(id, self.file_id).await?;
        let mut decrypted_file = AudioDecrypt::new(Some(key), encrypted_file);
        let offset = if AudioFiles::is_ogg_vorbis(self.format) { SPOTIFY_OGG_HEADER_END } else { 0 };
        decrypted_file.seek(SeekFrom::Start(offset))?;
        Ok(decrypted_file)
    }
}

pub struct TrackExt {
    pub id: SpotifyId,
    pub b62id: String,
    pub inner: Track,
    pub format: AudioFileFormatExt,
    pub file: TrackFileDescriptor,
}

impl TrackExt {
    pub fn new(ctx: &DLContext, inner: Track) -> Result<Self, Error> {
        let (id, b62id) = Self::id(&inner)?;
        let format = AudioFileFormatExt::new(&inner)?;
        let file = TrackFileDescriptor::new(ctx, &inner, format.format)?;
        let track = Self { id, b62id, inner, format, file };
        Ok(track)
    }

    pub fn id(track: &Track) -> Result<(SpotifyId, String), Error> {
        match track.id {
            SpotifyUri::Track { id } => Ok((id, id.to_base62()?)),
            _ => return Err(Error::failed_precondition("Invalid Track ID")),
        }
    }

    pub async fn save_audio(&self, ctx: &DLContext) -> Result<(), Error> {
        let mut decrypted_file = self.format.get_audio_file(ctx, self.id).await?;
        create_dir_all(self.file.path.parent().unwrap())?;
        let mut outfile = File::create(&self.file.path)?;
        copy(&mut decrypted_file, &mut outfile)?;
        info!("<{}-{}> Track Save \"{}\"", self.b62id, self.inner.number, self.inner.name);
        Ok(())
    }

    pub async fn deduplication_check(&self, ctx: &mut DLContext) -> Result<(), Error> {
        if let Some(inner) = self.inner.artists.get(0) {
            if let Ok(artist) = ArtistExt::new(ctx, inner.clone()) {
                artist.track(ctx);
            }
        }
        let occurrences = ctx.filedb.files.get(&self.file.base).cloned();
        if let Some(occurrences) = occurrences {
            let len = occurrences.len();
            if len > 2 {
                error!("<{}-{}> Track Multiple: \"{}\"", self.b62id, self.inner.number, self.inner.name);
                return Err(Error::already_exists("Multiple Occurences"));
            }
            for path in occurrences {
                let Some(path_str) = path.to_str() else {
                    continue;
                };
                let occurrence_single = path_str.contains(&ctx.config.singles_folder);
                let current_single = self.inner.album.album_type == AlbumType::SINGLE;
                if occurrence_single == current_single {
                    warn!("<{}-{}> Track Exists \"{}\"", self.b62id, self.inner.number, self.inner.name);
                    return Err(Error::already_exists("Album Version"));
                }
                if occurrence_single {
                    return match remove_file(&path) {
                        Ok(_) => {
                            ctx.filedb.remove_occurrence(&self.file.base, &path);
                            warn!("<{}-{}> Remove Duplicate \"{}\"", self.b62id, self.inner.number, path_str);
                            Ok(())
                        }
                        Err(_) => {
                            error!("<{}-{}> Remove Fail \"{}\"", self.b62id, self.inner.number, path_str);
                            Err(Error::unavailable("Remove Duplicate"))
                        }
                    };
                }
            }
            if len > 1 {
                return Err(Error::already_exists("Single Version"));
            }
        }
        Ok(())
    }

    pub async fn apply_tag(&self, ctx: &mut DLContext) -> Result<(), Error> {
        let mut tag = Tag::new(self.format.tag_type);
        tag.insert(TagItem::new(ItemKey::TrackTitle, ItemValue::Text(self.inner.name.clone())));
        tag.insert(TagItem::new(ItemKey::AlbumTitle, ItemValue::Text(self.inner.album.name.clone())));
        tag.insert(TagItem::new(ItemKey::TrackArtist, ItemValue::Text(artists_string(&self.inner))));
        tag.insert(TagItem::new(ItemKey::TrackNumber, ItemValue::Text(self.inner.number.to_string())));
        tag.insert(TagItem::new(ItemKey::Isrc, ItemValue::Text(self.inner.id.to_uri()?)));

        let cover = ctx.cover_cache.get_cover(ctx.session.http_client(), &self.inner).await?;
        let picture = Picture::new_unchecked(PictureType::CoverFront, Some(cover.mime.clone()), Some("cover".to_string()), cover.data.clone());
        tag.push_picture(picture);

        if let Err(e) = tag.save_to_path(&self.file.path, WriteOptions::default()) {
            warn!("<{}-{}> Metadata Fail \"{}\": {}", self.b62id, self.inner.number, self.inner.name, e);
        } else {
            debug!("<{}-{}> Metadata Write \"{}\"", self.b62id, self.inner.number, self.inner.name);
        }

        Ok(())
    }

    pub async fn download(&self, ctx: &mut DLContext) -> Result<(), Error> {
        if let Err(_) = self.deduplication_check(ctx).await {
            return Ok(());
        }
        self.save_audio(ctx).await?;
        self.apply_tag(ctx).await?;
        ctx.filedb.track_track(&self.file);
        Ok(())
    }
}
