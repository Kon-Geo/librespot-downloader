use std::{
    collections::HashMap, fs::{self, File, create_dir_all, remove_file}, io::{self, Seek, SeekFrom, copy}, path::{Path, PathBuf}, process::exit, sync::Arc
};
use librespot::{
    audio::{AudioDecrypt, AudioFile}, core::{
        Error, SpotifyId, SpotifyUri, authentication::Credentials, cache::Cache, config::SessionConfig, session::Session
    }, metadata::{
        Album, Artist, Metadata, Playlist, Track, album::AlbumType, artist::AlbumGroups, audio::{AudioFileFormat, AudioFiles}, image
    }, oauth::OAuthClientBuilder
};
use log::{LevelFilter, debug, error, info, warn};
use lofty::{
    config::WriteOptions,
    picture::{MimeType, Picture, PictureType},
    prelude::*,
    tag::{ItemKey, ItemValue, Tag, TagItem, TagType}
};
use http::{HeaderValue, Method, Request, header::ACCEPT};
use bytes::Bytes;
use sanitize_filename::sanitize;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    root_folder: String,
    singles_folder: String,
    default_genre: String,
    artist_genres: HashMap<String, String>,
    exclude: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root_folder: "downloads".to_string(),
            singles_folder: "Singles".to_string(),
            default_genre: "Generic".to_string(),
            artist_genres: HashMap::new(),
            exclude: Vec::new(),
        }
    }
}

const CONFIG: &str = "config.json";
const CACHE: &str = ".cache";
const CACHE_FILES: &str = ".cache/files";
const SPOTIFY_OGG_HEADER_END: u64 = 0xa7;
const IMAGE_URL: &str = "https://i.scdn.co/image/";
const FORMAT_PREFERENCE: [AudioFileFormat; 19] = [
    AudioFileFormat::FLAC_FLAC_24BIT,   // 1. Lossless, 24-bit high resolution
    AudioFileFormat::FLAC_FLAC,         // 2. Standard lossless FLAC
    AudioFileFormat::AAC_320,           // 3. High-bitrate AAC (excellent perceptual quality)
    AudioFileFormat::MP3_320,           // 4. Highest-bitrate MP3 (widely compatible)
    AudioFileFormat::MP3_256,           // 5. Mid-high MP3 bitrate
    AudioFileFormat::OGG_VORBIS_320,    // 6. High-quality Vorbis (slightly less efficient than AAC)
    AudioFileFormat::AAC_160,           // 7. Medium-bitrate AAC
    AudioFileFormat::MP3_160_ENC,       // 8. Possibly a special encoder variant, quality similar to MP3_160
    AudioFileFormat::MP3_160,           // 9. Standard MP3 midrange quality
    AudioFileFormat::OGG_VORBIS_160,    // 10. Mid-bitrate Vorbis
    AudioFileFormat::MP4_128,           // 11. Medium-low quality (likely AAC in MP4 container)
    AudioFileFormat::AAC_48,            // 12. Low-quality AAC variant
    AudioFileFormat::AAC_24,            // 13. Very low bitrate AAC
    AudioFileFormat::XHE_AAC_24,        // 14. xHE-AAC at 24 kbps — better compression than plain AAC_24
    AudioFileFormat::XHE_AAC_16,        // 15. Lower bitrate xHE-AAC
    AudioFileFormat::XHE_AAC_12,        // 16. Minimal bitrate, speech quality only
    AudioFileFormat::OGG_VORBIS_96,     // 17. Low-quality Vorbis
    AudioFileFormat::MP3_96,            // 18. Low-quality MP3
    AudioFileFormat::OTHER5,            // 19. Unknown/legacy format, last resort
];

fn get_extension_from_format(format: AudioFileFormat) -> &'static str {
    match format {
        AudioFileFormat::OGG_VORBIS_96
        | AudioFileFormat::OGG_VORBIS_160
        | AudioFileFormat::OGG_VORBIS_320 => "ogg",
        AudioFileFormat::MP3_96
        | AudioFileFormat::MP3_160
        | AudioFileFormat::MP3_256
        | AudioFileFormat::MP3_320
        | AudioFileFormat::MP3_160_ENC => "mp3",
        AudioFileFormat::AAC_24
        | AudioFileFormat::AAC_48
        | AudioFileFormat::AAC_160
        | AudioFileFormat::AAC_320
        | AudioFileFormat::MP4_128
        | AudioFileFormat::XHE_AAC_12
        | AudioFileFormat::XHE_AAC_16
        | AudioFileFormat::XHE_AAC_24 => "aac",
        AudioFileFormat::FLAC_FLAC | AudioFileFormat::FLAC_FLAC_24BIT => "flac",
        _ => "bin",
    }
}

fn format_data_rate(format: AudioFileFormat) -> usize {
    let kbps = match format {
        AudioFileFormat::OGG_VORBIS_96 => 12.,
        AudioFileFormat::OGG_VORBIS_160 => 20.,
        AudioFileFormat::OGG_VORBIS_320 => 40.,
        AudioFileFormat::MP3_256 => 32.,
        AudioFileFormat::MP3_320 => 40.,
        AudioFileFormat::MP3_160 => 20.,
        AudioFileFormat::MP3_96 => 12.,
        AudioFileFormat::MP3_160_ENC => 20.,
        AudioFileFormat::AAC_24 => 3.,
        AudioFileFormat::AAC_48 => 6.,
        AudioFileFormat::AAC_160 => 20.,
        AudioFileFormat::AAC_320 => 40.,
        AudioFileFormat::MP4_128 => 16.,
        AudioFileFormat::OTHER5 => 40.,
        AudioFileFormat::FLAC_FLAC => 112., // assume 900 kbit/s on average
        AudioFileFormat::XHE_AAC_12 => 1.5,
        AudioFileFormat::XHE_AAC_16 => 2.,
        AudioFileFormat::XHE_AAC_24 => 3.,
        AudioFileFormat::FLAC_FLAC_24BIT => 3.,
    };
    let data_rate: f32 = kbps * 1024.;
    data_rate.ceil() as usize
}

type FileOccurences = HashMap<String, Vec<PathBuf>>;

fn collect_file_occurences<F>(path: &Path, filter: &F) -> FileOccurences
where
    F: Fn(&str) -> Option<String>,
{
    let mut occurences = FileOccurences::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let file_path = entry.path();
            if file_path.is_dir() {
                let nested = collect_file_occurences(&file_path, filter);
                for (file, folders) in nested {
                    occurences
                        .entry(file)
                        .or_default()
                        .extend(folders);
                }
            } else if let Some(stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                if let Some(filtered_stem) = filter(stem) {
                    occurences
                        .entry(filtered_stem)
                        .or_default()
                        .push(file_path);
                }
            }
        }
    }
    occurences
}

fn remove_bracketed_content(input: &str) -> Option<String> {
    let mut result = String::new();
    let mut depth = 0;
    for c in input.chars() {
        match c {
            '(' | '[' => {
                depth += 1;
            }
            ')' | ']' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ => {
                if depth == 0 {
                    result.push(c);
                }
            }
        }
    }
    Some(result)
}

pub struct Cover {
    data: Vec<u8>,
    mime: MimeType,
}

pub struct Downloader {
    pub config: Arc<Config>,
    pub session: Arc<Session>,
    pub album_cover_cache: Arc<RwLock<HashMap<String, Arc<Cover>>>>,
    pub current_genre: Option<String>,
    pub current_occurences: FileOccurences,
}

impl Downloader {
    pub fn new(session: Arc<Session>) -> Self {
        let config = load_config().unwrap_or_else(|_| Config::default());
        Self {
            config: Arc::new(config),
            session,
            album_cover_cache: Arc::new(RwLock::new(HashMap::new())),
            current_genre: None,
            current_occurences: FileOccurences::new(),
        }
    }

    pub async fn download_from_stdin(&mut self) -> Result<(), Error> {
        info!("Enter a Spotify Track/Album/Playlist URL: ");
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();
        if input.is_empty() {
            return Ok(());
        }
        self.download_by_url(input).await
    }

    pub async fn download_by_url(&mut self, input: &str) -> Result<(), Error> {
        let url = input.trim();
        let path = url
            .split("open.spotify.com/")
            .nth(1)
            .ok_or(Error::unavailable("Invalid Spotify URL"))?;
        let mut parts = path.split('/');
        let kind = parts
            .next()
            .ok_or(Error::unavailable("Missing resource type"))?;
        let id = parts
            .next()
            .ok_or(Error::unavailable("Missing Spotify ID"))?
            .split('?')
            .next()
            .unwrap();
        let sp_id = SpotifyId::from_base62(id)?;
        match kind {
            "track" => self.download_track_by_id(sp_id).await,
            "album" => self.download_album_by_id(sp_id).await,
            "artist" => self.download_artist_by_id(sp_id).await,
            "playlist" => self.download_playlist_by_id(sp_id).await,
            _ => Err(Error::invalid_argument(format!("Unsupported URL: {}", kind)))
        }
    }

    pub async fn download_playlist_by_id(&mut self, id: SpotifyId) -> Result<(), Error> {
        let uri = SpotifyUri::Playlist { user: Some("".to_string()), id };
        let playlist = Playlist::get(&self.session, &uri).await?;
        self.download_playlist(playlist).await
    }

    pub async fn download_playlist(&mut self, album: Playlist) -> Result<(), Error> {
        info!("<{}> Playlist Download \"{}\"", album.id.to_id()?, album.name());
        self.download_tracks(album.tracks()).await
    }

    pub async fn download_artist_by_id(&mut self, id: SpotifyId) -> Result<(), Error> {
        let uri = SpotifyUri::Artist { id };
        let artist = Artist::get(&self.session, &uri).await?;
        self.download_artist(artist).await
    }

    pub async fn download_artist(&mut self, artist: Artist) -> Result<(), Error> {
        info!("<{}> Artist Download \"{}\"", artist.id.to_id()?, artist.name);
        let genre = self.get_artist_genre(Some(&artist));
        self.current_genre = Some(genre.clone());
        let mut dirpath = PathBuf::from(self.config.root_folder.clone());
        dirpath.push(genre);
        dirpath.push(artist.name.clone());
        self.current_occurences = collect_file_occurences(&dirpath, &remove_bracketed_content);
        self.download_albums(artist.albums).await?;
        self.download_albums(artist.singles).await?;
        self.current_genre = None;
        Ok(())
    }

    pub async fn download_album_by_id(&mut self, id: SpotifyId) -> Result<(), Error> {
        self.download_album_by_uri(&SpotifyUri::Album { id }).await
    }

    pub async fn download_album_by_uri(&mut self, uri: &SpotifyUri) -> Result<(), Error> {
        let album = Album::get(&self.session, uri).await?;
        self.download_album(album).await
    }

    pub async fn download_albums(&mut self, albums: AlbumGroups) -> Result<(), Error> {
        for uri in albums.current_releases() {
            self.download_album_by_uri(uri).await?;
        };
        Ok(())
    }

    pub async fn download_album(&mut self, album: Album) -> Result<(), Error> {
        let b62id = album.id.to_id()?;
        if self.config.exclude.contains(&b62id) {
            warn!("<{}> Album Skip/Exclude \"{}\"", b62id, album.name);
            Ok(())
        } else {
            if album.album_type != AlbumType::SINGLE {
                info!("<{}> Album Download \"{}\"", b62id, album.name);
            }
            self.download_tracks(album.tracks()).await
        }
    }

    async fn download_tracks(&mut self, tracks: impl Iterator<Item = &SpotifyUri>) -> Result<(), Error> {
        for track_uri in tracks {
            let track = Track::get(&self.session, track_uri).await?;
            let b62id = track.id.to_id()?;
            if self.config.exclude.contains(&b62id) {
                warn!("<{}> Track Skip/Exclude \"{}\"", b62id, track.name);
            } else if let Err(e) = self.download_track(&track).await {
                error!("<{}> Track Fail {} ({:?})", b62id, track.name, e);
            };
        };
        Ok(())
    }

    pub async fn download_track_by_id(&mut self, id: SpotifyId) -> Result<(), Error> {
        let uri = SpotifyUri::Track { id };
        let track = Track::get(&self.session, &uri).await?;
        self.download_track(&track).await
    }

    fn get_artist_genre(&self, artist: Option<&Artist>) -> String {
        self.current_genre.clone().unwrap_or_else(|| {
            if let Some(b62id) = artist.and_then(|a| a.id.to_id().ok()) {
                self.config.artist_genres
                    .get(&b62id)
                    .unwrap_or(&self.config.default_genre)
                    .clone()
            } else {
                self.config.default_genre.clone()
            }
        })
    }

    fn get_track_path(&self, track: &Track) -> PathBuf {
        let mut dirpath = PathBuf::from(self.config.root_folder.clone());
        if let Some(artist) = track.album.artists.0.get(0) {
            dirpath.push(self.get_artist_genre(Some(artist)));
            dirpath.push(artist.name.clone());
        }
        if track.album.album_type == AlbumType::SINGLE {
            dirpath.push(self.config.singles_folder.clone());
        } else {
            dirpath.push(&track.album.name);
        }
        dirpath
    }

    pub async fn download_track(&mut self, track: &Track) -> Result<(), Error> {
        let (track_id, b62id) = match track.id {
            SpotifyUri::Track { id } => (id, id.to_base62()?),
            _ => return Ok(()),
        };
        let artists_string = track.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(" & ");
        let track_basename = format!("{} - {} ", artists_string, track.name);
        let file_stem = format!("{}({})", &track_basename, b62id);
        let dirpath = self.get_track_path(track);

        if let Some(occurences) = self.current_occurences.get(&track_basename) {
            if occurences.len() > 2 {
                warn!("<{}-{}> Track Skip/Multiple: \"{}\"", b62id, track.number, track.name);
                return Ok(())
            } else if occurences.len() > 0 {
                for path in occurences {
                    if let Some(path_str) = path.to_str() {
                        let is_occurence_single = path_str.contains(&self.config.singles_folder);
                        let is_current_single = track.album.album_type == AlbumType::SINGLE;
                        if is_occurence_single == is_current_single {
                            warn!("<{}-{} > Track Skip/Exists \"{}\"", b62id, track.number, track.name);
                        } else if is_occurence_single {
                            match remove_file(&path) {
                                Ok(_) => warn!("<{}-{}> Track Remove/Duplicate \"{}\"", b62id, track.number, path_str),
                                Err(_) => error!("<{}-{}> Track Fail/Remove \"{}\"", b62id, track.number, path_str),
                            }
                        }
                    }
                }
            }
            if occurences.len() > 1 {
                return Ok(());
            }
        }

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

        let file_extension = get_extension_from_format(format);
        let filename = format!("{}.{}", file_stem, file_extension);
        let filename = sanitize(filename);
        let filename = filename.chars().take(200).collect::<String>();
        let filepath = dirpath.join(filename);
        if filepath.exists() {
            warn!("<{}-{}> Track Skip/Exists \"{}\"", b62id, track.number, track.name);
            return Ok(());
        }

        let bytes_per_second = format_data_rate(format);
        let encrypted_file = AudioFile::open(&self.session, *file_id, bytes_per_second).await?;
        let key = self.session.audio_key().request(track_id, *file_id).await?;
        let mut decrypted_file = AudioDecrypt::new(Some(key), encrypted_file);
        let offset = if AudioFiles::is_ogg_vorbis(format) { SPOTIFY_OGG_HEADER_END } else { 0 };
        decrypted_file.seek(SeekFrom::Start(offset))?;

        create_dir_all(dirpath)?;
        let mut outfile = File::create(&filepath)?;
        copy(&mut decrypted_file, &mut outfile)?;
        info!("<{}-{}> Track Save \"{:?}\"", b62id, track.number, filepath);
        
        let tag_type = match file_extension {
            "ogg" | "flac" => TagType::VorbisComments,
            _ => TagType::Id3v2,
        };
        self.apply_tag(track, tag_type, &filepath, artists_string).await?;
        Ok(())
    }

    async fn apply_tag(&mut self, track: &Track, tag_type: TagType, filepath: &Path, artists_string: String) -> Result<(), Error> {
        let b62id = track.id.to_id()?;
        let mut tag = Tag::new(tag_type);
        tag.insert(TagItem::new(ItemKey::TrackTitle, ItemValue::Text(track.name.clone())));
        tag.insert(TagItem::new(ItemKey::AlbumTitle, ItemValue::Text(track.album.name.clone())));
        tag.insert(TagItem::new(ItemKey::TrackArtist, ItemValue::Text(artists_string)));
        tag.insert(TagItem::new(ItemKey::TrackNumber, ItemValue::Text(track.number.to_string())));
        tag.insert(TagItem::new(ItemKey::Isrc, ItemValue::Text(track.id.to_uri()?)));

        let cover = self.get_cover(track).await?;
        let picture = Picture::new_unchecked(PictureType::CoverFront, Some(cover.mime.clone()), Some("cover".to_string()), cover.data.clone());
        tag.push_picture(picture);

        if let Err(e) = tag.save_to_path(&filepath, WriteOptions::default()) {
            warn!("<{}-{}> Track Metadata/Fail \"{:?}\": {}", b62id, track.number, filepath, e);
        } else {
            debug!("<{}-{}> Metadata written to {:?}", b62id, track.number, filepath);
        }

        Ok(())
    }

    async fn get_cover(&mut self, track: &Track) -> Result<Arc<Cover>, Error> {
        fn size_rank(size: image::ImageSize) -> i32 {
            match size {
                image::ImageSize::DEFAULT => 0,
                image::ImageSize::SMALL => 1,
                image::ImageSize::LARGE => 2,
                image::ImageSize::XLARGE => 3,
            }
        }
        let cover = track.album.covers
            .iter()
            .max_by_key(|c| size_rank(c.size))
            .ok_or_else(|| Error::failed_precondition("Album has no cover"))?;
        let cover_id = cover.id.to_string();
        {
            let cache = self.album_cover_cache.read().await;
            if let Some(cover) = cache.get(&cover_id) {
                return Ok(Arc::clone(cover));
            }
        }
        self.download_cover(&cover_id).await?;
        let cache = self.album_cover_cache.read().await;
        Ok(Arc::clone(cache.get(&cover_id).unwrap()))
    }   

    async fn download_cover(&mut self, id: &String) -> Result<(), Error> {
        let request = Request::builder()
            .method(&Method::GET)
            .uri(format!("{}{}", IMAGE_URL, id))
            .header(ACCEPT, HeaderValue::from_static("image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8"))
            .body(Bytes::new())?;
        let body = self.session.http_client().request_body(request).await?;
        let cover = Arc::new(Cover {
            data: body.to_vec(),
            mime: infer::get(&body)
                .map(|t| MimeType::from_str(t.mime_type()))
                .unwrap_or(MimeType::Jpeg),
        });
        let mut cache = self.album_cover_cache.write().await;
        cache.insert(id.to_owned(), cover);
        Ok(())
    }
}

fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let contents = fs::read_to_string(CONFIG)?;
    let config: Config = serde_json::from_str(&contents)?;
    Ok(config)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::builder()
        .filter_module("librespot", LevelFilter::Info)
        .init();

    let session_config = SessionConfig::default();

    let cache = Cache::new(Some(CACHE), Some(CACHE), Some(CACHE_FILES), None)?;
    let credentials = cache
        .credentials()
        .ok_or(Error::unavailable("credentials not cached"))
        .or_else(|_| {
            OAuthClientBuilder::new(
                &session_config.client_id,
                "http://127.0.0.1:8898/login",
                vec!["streaming"],
            )
            .open_in_browser()
            .build()?
            .get_access_token()
            .map(|t| Credentials::with_access_token(t.access_token))
        })?;

    info!("Connecting...");
    let session = Session::new(session_config, Some(cache));
    if let Err(e) = session.connect(credentials, true).await {
        error!("Error connecting: {}", e);
        exit(1);
    }
    
    let mut downloader = Downloader::new(Arc::new(session));

    while downloader.download_from_stdin().await.is_ok() {}

    Ok(())
}
