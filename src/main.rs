use std::{
    collections::HashMap,
    fs::{File, create_dir_all},
    io::{self, Read, Seek, SeekFrom, copy},
    path::PathBuf,
    process::exit
};
use librespot::{
    core::{
        Error, authentication::Credentials, cache::Cache, config::SessionConfig, session::Session,
        SpotifyId, SpotifyUri
    },
    audio::{AudioDecrypt, AudioFile},
    metadata::{
        Album, Metadata, Track, image,
        audio::{AudioFileFormat, AudioFiles},
    },
    oauth::OAuthClientBuilder
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

fn get_extension_from_format(format: AudioFileFormat) -> String {
    let extension = match format {
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
    };
    String::from(extension)
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

struct Subfile<T: Read + Seek> {
    stream: T,
    offset: u64,
    length: u64,
}

impl<T: Read + Seek> Subfile<T> {
    pub fn new(mut stream: T, offset: u64, length: u64) -> Result<Subfile<T>, io::Error> {
        let target = SeekFrom::Start(offset);
        stream.seek(target)?;

        Ok(Subfile {
            stream,
            offset,
            length,
        })
    }
}

impl<T: Read + Seek> Read for Subfile<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

impl<T: Read + Seek> Seek for Subfile<T> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let pos = match pos {
            SeekFrom::Start(offset) => SeekFrom::Start(offset + self.offset),
            SeekFrom::End(offset) => {
                if (self.length as i64 - offset) < self.offset as i64 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "newpos would be < self.offset",
                    ));
                }
                pos
            }
            _ => pos,
        };

        let newpos = self.stream.seek(pos)?;
        Ok(newpos - self.offset)
    }
}

pub struct Downloader {
    pub session: Session,
    album_cover_cache: HashMap<String, (Vec<u8>, MimeType)>,
}

impl Downloader {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            album_cover_cache: HashMap::new(),
        }
    }

    pub async fn download_album_by_id(&mut self, base62: &str, directory: &str) -> Result<(), Error> {
        let id = SpotifyId::from_base62(base62)?;
        let uri = SpotifyUri::Album { id };
        let album = Album::get(&self.session, &uri).await?;
        self.download_album(album, directory).await
    }

    pub async fn download_album(&mut self, album: Album, directory: &str) -> Result<(), Error> {
        info!("Downloading Album: {}", album.name);
        let mut dirpath = PathBuf::from(directory);
        dirpath.push(&album.name);
        info!("<{}> saved at {:?}", album.id, dirpath);
        _ = create_dir_all(&dirpath);
        for track_uri in album.tracks() {
            self.download_track_by_uri(&track_uri, &dirpath).await?;
        };
        Ok(())
    }

    pub async fn download_track_by_uri(&mut self, uri: &SpotifyUri, dirpath: &PathBuf) -> Result<(), Error> {
        let track = Track::get(&self.session, uri).await?;
        self.download_track(&track, dirpath).await
    }

    pub async fn download_track(&mut self, track: &Track, dirpath: &PathBuf) -> Result<(), Error> {
        info!("Downloading Track #{}: {} ({})", track.number, track.name, track.id);
        let track_id = match track.id {
            SpotifyUri::Track { id } => id,
            _ => return Ok(()),
        };

        track.files.iter().for_each(|file| {
            debug!("<{}> has format {:?}", track.id, file.0);
        });

        let (format, file_id) = match FORMAT_PREFERENCE
            .iter()
            .find_map(|format| {
                track.files
                .get(format)
                .map(|file_id| (*format, file_id.clone()))
            })
        {
            Some(format) => {
                debug!("<{}> selected format {:?}", track.id, &format.0);
                format
            },
            None => {
                warn!("<{}> is not available in any supported format", track.id);
                return Ok(());
            }
        };
        let bytes_per_second = format_data_rate(format);
        let encrypted_file = AudioFile::open(&self.session, file_id, bytes_per_second);
        let encrypted_file = match encrypted_file.await {
            Ok(encrypted_file) => encrypted_file,
            Err(e) => {
                error!("Unable to load encrypted file: {e:?}");
                return Ok(());
            }
        };
        let stream_loader_controller = encrypted_file.get_stream_loader_controller()?;
        let key = match self.session.audio_key().request(track_id, file_id).await {
            Ok(key) => Some(key),
            Err(e) => {
                warn!("Unable to load key, continuing without decryption: {e}");
                None
            }
        };
        let decrypted_file = AudioDecrypt::new(key, encrypted_file);
        let offset = if AudioFiles::is_ogg_vorbis(format) { SPOTIFY_OGG_HEADER_END } else { 0 };
        let mut audio_file = match Subfile::new(decrypted_file, offset, stream_loader_controller.len() as u64) {
            Ok(audio_file) => audio_file,
            Err(e) => {
                error!("PlayerTrackLoader::download_track error opening subfile: {e}");
                return Ok(());
            }
        };
        self.save_decrypted_audio(format, &track, &mut audio_file, &dirpath).await?;
        Ok(())
    }

    async fn save_decrypted_audio(
        &mut self,
        format: AudioFileFormat,
        track: &Track,
        audio_file: &mut Subfile<AudioDecrypt<AudioFile>>,
        dirpath: &PathBuf
    ) -> Result<(), Error> {
        let file_extension = get_extension_from_format(format);
        let artists = track.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(" & ");
        
        let mut filepath = dirpath.clone();
        let filename = format!("{} - {} ({}).{}", artists, track.name, track.id.to_id()?, file_extension);
        filepath.push(filename);
        let mut outfile = File::create(&filepath)?;
        copy(audio_file, &mut outfile)?;
        info!("Decrypted content saved to {:?}", filepath);

        self.apply_tag(file_extension, track, artists, filepath).await?;

        Ok(())
    }

    async fn apply_tag(
        &mut self,
        file_extension: String,
        track: &Track,
        artists: String,
        filepath: PathBuf
    ) -> Result<(), Error> {
        let tag_type = match file_extension.as_str() {
            "ogg" | "flac" => TagType::VorbisComments,
            _ => TagType::Id3v2,
        };
        let mut tag = Tag::new(tag_type);
        tag.insert(TagItem::new(ItemKey::TrackTitle, ItemValue::Text(track.name.clone())));
        tag.insert(TagItem::new(ItemKey::AlbumTitle, ItemValue::Text(track.album.name.clone())));
        tag.insert(TagItem::new(ItemKey::TrackArtist, ItemValue::Text(artists)));
        tag.insert(TagItem::new(ItemKey::TrackNumber, ItemValue::Text(track.number.to_string())));
        tag.insert(TagItem::new(ItemKey::Isrc, ItemValue::Text(track.id.to_uri()?)));
        // tag.insert(TagItem::new(ItemKey::RecordingDate, ItemValue::Text(year)));

        let (cover_data, mime_type) = self.get_cover(track).await?;
        let picture = Picture::new_unchecked(
            PictureType::CoverFront,
            Some(mime_type),
            Some("cover".to_string()),
            cover_data
        );
        tag.push_picture(picture);

        if let Err(e) = tag.save_to_path(&filepath, WriteOptions::default()) {
            warn!("Unable to write metadata to {:?}: {}", filepath, e);
        } else {
            debug!("Metadata written to {:?}", filepath);
        }

        Ok(())
    }

    async fn get_cover(&mut self, track: &Track) -> Result<(Vec<u8>, MimeType), Error> {
        fn size_rank(size: image::ImageSize) -> i32 {
            match size {
                image::ImageSize::DEFAULT => 0,
                image::ImageSize::SMALL => 1,
                image::ImageSize::LARGE => 2,
                image::ImageSize::XLARGE => 3,
            }
        }
        let cover = track.album.covers.iter().max_by_key(|cover| size_rank(cover.size)).unwrap();
        let cover_id = cover.id.to_string();
        if !self.album_cover_cache.contains_key(&cover_id) {
            self.download_cover(&cover_id).await?;
        }
        let (cover_data, mime_type) = self.album_cover_cache.get(&cover_id).unwrap();
        Ok((cover_data.clone(), mime_type.clone()))
    }   

    async fn download_cover(&mut self, id: &String) -> Result<(), Error> {
        let request = Request::builder()
            .method(&Method::GET)
            .uri(format!("{}{}", IMAGE_URL, id))
            .header(ACCEPT, HeaderValue::from_static("image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8"))
            .body(Bytes::new())?;
        let body = self.session.http_client().request_body(request).await?;
        let cover_data = body.to_vec();
        let mime_type = infer::get(&cover_data)
            .map(|t| MimeType::from_str(t.mime_type()))
            .unwrap_or(MimeType::Jpeg);
        self.album_cover_cache.insert(id.clone(), (cover_data, mime_type));
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::builder()
        .filter_module("librespot", LevelFilter::Debug)
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
        info!("Error connecting: {e}");
        exit(1);
    }
    
    let mut downloader = Downloader::new(session);
    downloader.download_album_by_id("2FRgTjahtyzUQG8A3ZaaDT", "downloads").await?;

    Ok(())
}
