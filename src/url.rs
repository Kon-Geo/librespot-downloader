use std::io;
use librespot::{core::{Error, SpotifyId}, metadata::{Album, Artist, Playlist, Track}};
use log::{error, info};

use crate::{ctx::DLContext, download::GetById};

/// Reads one line from stdin and downloads it.
/// Returns `Ok(false)` when the user is done (EOF or a quit command).
/// Download errors are logged and do not end the session.
pub async fn download_from_stdin(ctx: &mut DLContext) -> Result<bool, Error> {
    info!("Enter a Spotify Track/Album/Playlist URL (or 'q' to quit): ");
    let mut input = String::new();
    if io::stdin().read_line(&mut input)? == 0 {
        return Ok(false);
    }
    let input = input.trim();
    match input {
        "" => Ok(true),
        "q" | "quit" | "exit" => Ok(false),
        _ => {
            if let Err(e) = download_by_url(ctx, input).await {
                error!("Download Fail \"{}\": {}", input, e);
            }
            Ok(true)
        }
    }
}

pub fn parse_url(input: &str) -> Result<(&str, SpotifyId), Error> {
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
    Ok((kind, sp_id))
}

pub async fn download_by_url(ctx: &mut DLContext, input: &str) -> Result<(), Error> {
    let (kind, sp_id) = parse_url(input)?;
    match kind {
        "track" => Track::download_by_id(ctx, sp_id).await,
        "album" => Album::download_by_id(ctx, sp_id).await,
        "artist" => Artist::download_by_id(ctx, sp_id).await,
        "playlist" => Playlist::download_by_id(ctx, sp_id).await,
        _ => Err(Error::invalid_argument(format!("Unsupported URL: {}", kind)))
    }
}
