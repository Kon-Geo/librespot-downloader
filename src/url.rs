use std::io;
use librespot::{core::{Error, SpotifyId}, metadata::{Album, Artist, Playlist, Track}};
use log::info;

use crate::{ctx::DLContext, download::GetById};

pub async fn download_from_stdin(ctx: &mut DLContext) -> Result<(), Error> {
    info!("Enter a Spotify Track/Album/Playlist URL: ");
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    if input.is_empty() {
        return Ok(());
    }
    download_by_url(ctx, input).await
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
