use librespot::core::Session;
use log::warn;
use crate::{config::{CONFIG, Config, load_config}, cover::CoverCache, fs::FileDB};

#[derive(Debug, Default)]
pub struct Stats {
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: usize,
}

pub struct DLContext {
    pub stats: Stats,
    pub config: Config,
    pub session: Session,
    pub cover_cache: CoverCache,
    pub filedb: FileDB,
}

impl DLContext {
    pub fn new(session: Session) -> Self {
        let config = load_config().unwrap_or_else(|e| {
            warn!("Config Fail \"{}\": {} (using defaults)", CONFIG, e);
            Config::default()
        });
        Self {
            stats: Stats::default(),
            config,
            session,
            cover_cache: CoverCache::new(),
            filedb: FileDB::new(),
        }
    }
}
