use librespot::core::Error;
use librespot_downloader::{ctx::DLContext, session::setup_session, url::download_from_stdin};
use log::{LevelFilter};

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::builder().filter_module("librespot", LevelFilter::Info).init();

    let session = setup_session().await?;

    let mut downloader = DLContext::new(session);

    while download_from_stdin(&mut downloader).await.is_ok() {}

    Ok(())
}
