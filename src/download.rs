use librespot::{core::{Error, SpotifyId, SpotifyUri}, metadata::{Album, Artist, Metadata, Playlist, Track, album::AlbumType, artist::AlbumGroups}};
use log::{debug, error, warn};
use crate::{ctx::DLContext, metadata::{TrackExt}};

pub(crate) trait Downloadable {
    async fn download(&self, ctx: &mut DLContext) -> Result<(), Error>;
}

pub(crate) trait GetById<S: Metadata + Downloadable> {
    fn uri_from_id(id: SpotifyId) -> SpotifyUri;
    async fn get_by_uri(ctx: &mut DLContext, uri: &SpotifyUri) -> Result<S, Error> {
        S::get(&ctx.session, &uri).await
    }
    async fn get_by_id(ctx: &mut DLContext, id: SpotifyId) -> Result<S, Error> {
        Self::get_by_uri(ctx, &Self::uri_from_id(id)).await
    }
    async fn download_by_id(ctx: &mut DLContext, id: SpotifyId) -> Result<(), Error> {
        Self::get_by_id(ctx, id).await?.download(ctx).await
    }
}

impl GetById<Playlist> for Playlist {
    fn uri_from_id(id: SpotifyId) -> SpotifyUri {
        SpotifyUri::Playlist { user: Some("".to_string()), id }
    }
}

impl GetById<Artist> for Artist {
    fn uri_from_id(id: SpotifyId) -> SpotifyUri {
        SpotifyUri::Artist { id }
    }
}

impl GetById<Album> for Album {
    fn uri_from_id(id: SpotifyId) -> SpotifyUri {
        SpotifyUri::Album { id }
    }
}

impl GetById<Track> for Track {
    fn uri_from_id(id: SpotifyId) -> SpotifyUri {
        SpotifyUri::Track { id }
    }
}

impl Downloadable for Playlist {
    async fn download(&self, ctx: &mut DLContext) -> Result<(), Error> {
        debug!("<{}> Playlist Download \"{}\"", self.id.to_id()?, self.name());
        self.tracks().collect::<Vec<_>>().download(ctx).await
    }
}

impl Downloadable for Artist {
    async fn download(&self, ctx: &mut DLContext) -> Result<(), Error> {
        debug!("<{}> Artist Download \"{}\"", self.id.to_id()?, self.name);
        self.albums.download(ctx).await?;
        self.singles.download(ctx).await?;
        Ok(())
    }
}

impl Downloadable for Album {
    async fn download(&self, ctx: &mut DLContext) -> Result<(), Error> {
        let b62id = self.id.to_id()?;
        if ctx.config.exclude.contains_key(&b62id) {
            warn!("<{}> Album Exclude \"{}\"", b62id, self.name);
            Ok(())
        } else {
            if self.album_type != AlbumType::SINGLE {
                debug!("<{}> Album Download \"{}\"", b62id, self.name);
            }
            self.tracks().collect::<Vec<_>>().download(ctx).await
        }
    }
}

impl Downloadable for AlbumGroups {
    async fn download(&self, ctx: &mut DLContext) -> Result<(), Error> {
        for uri in self.current_releases() {
            let album = Album::get_by_uri(ctx, uri).await?;
            album.download(ctx).await?;
        };
        Ok(())
    }
}

impl Downloadable for Track {
    async fn download(&self, ctx: &mut DLContext) -> Result<(), Error> {
        TrackExt::new(ctx, self.clone())?
            .download()
            .await
    }
}

impl Downloadable for Vec<&SpotifyUri> {
    async fn download(&self, ctx: &mut DLContext) -> Result<(), Error> {
        for track_uri in self {
            let track = Track::get(&ctx.session, track_uri).await?;
            let b62id = track.id.to_id()?;
            if ctx.config.exclude.contains_key(&b62id) {
                warn!("<{}> Track Exclude \"{}\"", b62id, track.name);
            } else if let Err(e) = track.download(ctx).await {
                error!("<{}> Track Fail {} ({:?})", b62id, track.name, e);
            };
        };
        Ok(())
    }
}
