use std::{collections::HashMap, fs};

use librespot::metadata::audio::AudioFileFormat;
use lofty::tag::TagType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub root_folder: String,
    pub singles_folder: String,
    pub default_genre: String,
    pub artist_genres: HashMap<String, Vec<String>>,
    pub exclude: HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root_folder: "downloads".to_string(),
            singles_folder: "Singles".to_string(),
            default_genre: "Generic".to_string(),
            artist_genres: HashMap::new(),
            exclude: HashMap::new(),
        }
    }
}

pub const CONFIG: &str = "config.json";
pub const CACHE: &str = ".cache";
pub const CACHE_FILES: &str = ".cache/files";
pub const SPOTIFY_OGG_HEADER_END: u64 = 0xa7;
pub const IMAGE_URL: &str = "https://i.scdn.co/image/";
pub const FORMAT_PREFERENCE: [AudioFileFormat; 19] = [
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

pub fn get_extension_from_format(format: AudioFileFormat) -> &'static str {
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

pub fn get_extension_rank(extension: &str) -> i32 {
    match extension {
        "m4a" => 5,
        "flac" => 4,
        "ogg" => 3,
        "aac" => 2,
        "mp3" => 1,
        _ => 0,
    }
}

pub fn format_data_rate(format: AudioFileFormat) -> usize {
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

pub fn format_tag_type(format: AudioFileFormat) -> TagType {
    match get_extension_from_format(format) {
        "ogg" | "flac" => TagType::VorbisComments,
        _ => TagType::Id3v2,
    }
}

pub fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let contents = fs::read_to_string(CONFIG)?;
    let config: Config = serde_json::from_str(&contents)?;
    Ok(config)
}
