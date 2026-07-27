use librespot::core::Session;
use crate::{config::{Config, load_config}, cover::CoverCache, fs::FileDB};

pub struct DLContext {
    pub config: Config,
    pub session: Session,
    pub cover_cache: CoverCache,
    pub filedb: FileDB,
}

impl DLContext {
    pub fn new(session: Session) -> Self {
        let config = load_config().unwrap_or_else(|_| Config::default());
        Self {
            config,
            session,
            cover_cache: CoverCache::new(),
            filedb: FileDB::new(),
        }
    }
}
