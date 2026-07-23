use std::{collections::HashMap, sync::{Arc, RwLock}};
use librespot::core::Session;
use crate::{config::{Config, load_config}, cover::Cover, fs::FileOccurences};

pub struct DLContext {
    pub config: Arc<Config>,
    pub session: Arc<Session>,
    pub covers: Arc<RwLock<HashMap<String, Arc<Cover>>>>,
    pub genre: Option<String>,
    pub occurences: FileOccurences,
}

impl DLContext {
    pub fn new(session: Arc<Session>) -> Self {
        let config = load_config().unwrap_or_else(|_| Config::default());
        Self {
            config: Arc::new(config),
            session,
            covers: Arc::new(RwLock::new(HashMap::new())),
            genre: None,
            occurences: FileOccurences::new(),
        }
    }
}
