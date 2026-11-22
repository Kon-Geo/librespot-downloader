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
            for url in input.split_whitespace() {
                if let Err(e) = download_by_url(ctx, url).await {
                    error!("Download Fail \"{}\": {}", url, e);
                }
            }
            Ok(true)
        }
    }
}

/// Accepts `https://open.spotify.com/[intl-xx/][user/<name>/]<kind>/<id>[?...]`
/// and `spotify:<kind>:<id>` (including `spotify:user:<name>:playlist:<id>`).
pub fn parse_url(input: &str) -> Result<(&str, SpotifyId), Error> {
    let input = input.trim();
    let (kind, id) = if let Some(uri) = input.strip_prefix("spotify:") {
        let parts: Vec<&str> = uri.split(':').collect();
        match parts.as_slice() {
            [.., kind, id] => (*kind, *id),
            _ => return Err(Error::unavailable("Invalid Spotify URI")),
        }
    } else {
        let path = input
            .split("open.spotify.com/")
            .nth(1)
            .ok_or(Error::unavailable("Invalid Spotify URL"))?;
        let path = path.split(['?', '#']).next().unwrap_or("");
        let mut parts = path
            .split('/')
            .filter(|p| !p.is_empty())
            .skip_while(|p| p.starts_with("intl-") || *p == "embed");
        let mut kind = parts.next().ok_or(Error::unavailable("Missing resource type"))?;
        if kind == "user" {
            parts.next();
            kind = parts.next().ok_or(Error::unavailable("Missing resource type"))?;
        }
        let id = parts.next().ok_or(Error::unavailable("Missing Spotify ID"))?;
        (kind, id)
    };
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

#[cfg(test)]
mod tests {
    use super::parse_url;

    const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";

    fn kind(input: &str) -> Option<String> {
        parse_url(input).ok().map(|(k, _)| k.to_string())
    }

    #[test]
    fn parses_url_variants() {
        assert_eq!(kind(&format!("https://open.spotify.com/track/{ID}?si=abc")).as_deref(), Some("track"));
        assert_eq!(kind(&format!("https://open.spotify.com/intl-de/album/{ID}")).as_deref(), Some("album"));
        assert_eq!(kind(&format!("https://open.spotify.com/user/bob/playlist/{ID}")).as_deref(), Some("playlist"));
        assert_eq!(kind(&format!("spotify:track:{ID}")).as_deref(), Some("track"));
        assert_eq!(kind(&format!("spotify:user:bob:playlist:{ID}")).as_deref(), Some("playlist"));
        assert!(parse_url("https://example.com/track/x").is_err());
    }
}
