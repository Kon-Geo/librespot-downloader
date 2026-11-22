use std::{fs::{File, create_dir_all, remove_file, rename}, io::{Seek, SeekFrom, copy}, path::PathBuf};
use librespot::{audio::{AudioDecrypt, AudioFile}, core::{Error, FileId, SpotifyId, SpotifyUri}, metadata::{Artist, Track, album::AlbumType, audio::{AudioFileFormat, AudioFiles}}};
use log::{error, info, warn, debug};
use sanitize_filename::sanitize;
use lofty::{config::WriteOptions, picture::{Picture, PictureType}, tag::{ItemKey, ItemValue, Tag, TagExt, TagItem, TagType}};
use crate::{config::{FORMAT_PREFERENCE, SPOTIFY_OGG_HEADER_END, format_data_rate, format_tag_type, get_extension_from_format, get_extension_rank}, ctx::DLContext};

const MAX_FILE_NAME_LEN: usize = 200;

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

#[derive(Clone)]
pub struct TrackFileDescriptor {
    pub base: String,
    pub stem: String,
    pub extension: String,
    pub name: String,
    pub path: PathBuf,
}

impl TrackFileDescriptor {
    pub fn new(ctx: &DLContext, track: &Track, format: AudioFileFormat) -> Result<Self, Error> {
        let b62id = track.id.to_id()?;
        let basepath = Self::get_path(ctx, &track);
        let extension = get_extension_from_format(format).to_string();
        let suffix = format!(" ({}).{}", b62id, extension);
        let base = sanitize(format!("{} - {}", artists_string(track), track.name));
        // Truncate only the readable part so the id and extension always survive.
        let base = base.chars().take(MAX_FILE_NAME_LEN.saturating_sub(suffix.chars().count())).collect::<String>();
        let base = base.trim_end().to_string();
        let stem = format!("{} ({})", base, b62id);
        let name = format!("{}.{}", stem, extension);
        let path = basepath.join(&name);
        let file = Self { base, stem, extension, name, path };
        Ok(file)
    }

    pub fn get_path(ctx: &DLContext, track: &Track) -> PathBuf {
        let mut dirpath = PathBuf::from(ctx.config.root_folder.clone());
        if let Some(artist) = track.album.artists.0.get(0) {
            dirpath.push(get_artist_genre(ctx, Some(artist)));
            dirpath.push(sanitize(artist.name.clone()));
        }
        if track.album.album_type == AlbumType::SINGLE {
            dirpath.push(ctx.config.singles_folder.clone());
        } else {
            dirpath.push(sanitize(&track.album.name));
        }
        dirpath
    }

    pub fn get_score(&self, singles_folder: &str) -> i32 {
        let mut score: i32 = 0;
        if let Some(extension) = self.path.extension().and_then(|s| s.to_str()) {
            score += get_extension_rank(extension);
        }
        // Being outside the singles folder outweighs any format difference.
        let in_singles = self.path.components().any(|c| c.as_os_str() == singles_folder);
        if !in_singles {
            score += 100;
        }
        score
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
        // Write to a temporary file so an interrupted download never looks like a finished track.
        let mut part_name = self.file.path.clone().into_os_string();
        part_name.push(".part");
        let part_path = PathBuf::from(part_name);
        let result = File::create(&part_path)
            .and_then(|mut outfile| copy(&mut decrypted_file, &mut outfile))
            .and_then(|_| rename(&part_path, &self.file.path));
        if let Err(e) = result {
            let _ = remove_file(&part_path);
            return Err(e.into());
        }
        info!("<{}-{}> Track Save \"{}\"", self.b62id, self.inner.number, self.inner.name);
        Ok(())
    }

    pub async fn deduplication_check(&self, ctx: &mut DLContext) -> bool {
        if let Some(inner) = self.inner.artists.get(0) {
            if let Ok(artist) = ArtistExt::new(ctx, inner.clone()) {
                artist.track(ctx);
            }
        }
        let mut download = false;
        let Some(occurrences) = ctx.filedb.files.get(&self.file.base).cloned() else {
            return true;
        };
        let current_score = self.file.get_score(&ctx.config.singles_folder);
        for file in occurrences {
            let file_score = file.get_score(&ctx.config.singles_folder);
            if current_score <= file_score { continue; }
            download = true;
            match remove_file(&file.path) {
                Ok(_) => {
                    ctx.filedb.remove_occurrence(&self.file.base, &file.path);
                    warn!("<{}-{}> Remove Duplicate \"{:?}\"", self.b62id, self.inner.number, file.path);
                }
                Err(_) => {
                    error!("<{}-{}> Remove Fail \"{:?}\"", self.b62id, self.inner.number, file.path);
                }
            };
        }
        if !download {
            ctx.stats.skipped += 1;
            warn!("<{}-{}> Track Exists \"{}\"", self.b62id, self.inner.number, self.inner.name);
        }
        download
    }

    pub async fn apply_tag(&self, ctx: &mut DLContext) -> Result<(), Error> {
        let mut tag = Tag::new(self.format.tag_type);
        tag.insert(TagItem::new(ItemKey::TrackTitle, ItemValue::Text(self.inner.name.clone())));
        tag.insert(TagItem::new(ItemKey::AlbumTitle, ItemValue::Text(self.inner.album.name.clone())));
        tag.insert(TagItem::new(ItemKey::TrackArtist, ItemValue::Text(artists_string(&self.inner))));
        tag.insert(TagItem::new(ItemKey::TrackNumber, ItemValue::Text(self.inner.number.to_string())));
        tag.insert(TagItem::new(ItemKey::DiscNumber, ItemValue::Text(self.inner.disc_number.to_string())));
        tag.insert(TagItem::new(ItemKey::Year, ItemValue::Text(self.inner.album.date.year().to_string())));
        if let Some(isrc) = self.inner.external_ids.iter().find(|e| e.external_type.eq_ignore_ascii_case("isrc")) {
            tag.insert(TagItem::new(ItemKey::Isrc, ItemValue::Text(isrc.id.clone())));
        }
        let album_artists = self.inner.album.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(" & ");
        if !album_artists.is_empty() {
            tag.insert(TagItem::new(ItemKey::AlbumArtist, ItemValue::Text(album_artists)));
        }
        // Only tag a genre when the artist is explicitly configured, not the "Generic" fallback.
        if let Some(b62id) = self.inner.album.artists.0.first().and_then(|a| a.id.to_id().ok()) {
            if ctx.config.artist_genres.contains_key(&b62id) {
                tag.insert(TagItem::new(ItemKey::Genre, ItemValue::Text(ArtistExt::get_genre(ctx, &b62id))));
            }
        }

        match ctx.cover_cache.get_cover(ctx.session.http_client(), &self.inner).await {
            Ok(cover) => {
                let picture = Picture::new_unchecked(PictureType::CoverFront, Some(cover.mime.clone()), Some("cover".to_string()), cover.data.clone());
                tag.push_picture(picture);
            }
            Err(e) => warn!("<{}-{}> Cover Fail \"{}\": {}", self.b62id, self.inner.number, self.inner.name, e),
        }

        if let Err(e) = tag.save_to_path(&self.file.path, WriteOptions::default()) {
            warn!("<{}-{}> Metadata Fail \"{}\": {}", self.b62id, self.inner.number, self.inner.name, e);
        } else {
            debug!("<{}-{}> Metadata Write \"{}\"", self.b62id, self.inner.number, self.inner.name);
        }

        Ok(())
    }

    pub async fn download(&self, ctx: &mut DLContext) -> Result<(), Error> {
        if !self.deduplication_check(ctx).await {
            return Ok(());
        }
        self.save_audio(ctx).await?;
        if let Err(e) = self.apply_tag(ctx).await {
            warn!("<{}-{}> Tag Fail \"{}\": {}", self.b62id, self.inner.number, self.inner.name, e);
        }
        ctx.filedb.track_track(self.file.clone());
        ctx.stats.downloaded += 1;
        Ok(())
    }
}
