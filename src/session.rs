use librespot::{core::{Error, Session, SessionConfig, cache::Cache}, discovery::Credentials, oauth::OAuthClientBuilder};

use crate::config::{CACHE, CACHE_FILES};

pub async fn setup_session() -> Result<Session, Error> {
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
    let session = Session::new(session_config, Some(cache));
    session.connect(credentials, true).await?;
    Ok(session)
}
