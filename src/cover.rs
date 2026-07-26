use std::sync::Arc;
use bytes::Bytes;
use http::{HeaderValue, Method, Request, header::ACCEPT};
use librespot::{core::Error, metadata::{Track, image}};
use lofty::picture::MimeType;
use crate::{config::IMAGE_URL, ctx::DLContext};

pub struct Cover {
    pub data: Vec<u8>,
    pub mime: MimeType,
}

pub async fn get_cover(ctx: &mut DLContext, track: &Track) -> Result<Arc<Cover>, Error> {
    fn size_rank(size: image::ImageSize) -> i32 {
        match size {
            image::ImageSize::DEFAULT => 0,
            image::ImageSize::SMALL => 1,
            image::ImageSize::LARGE => 2,
            image::ImageSize::XLARGE => 3,
        }
    }
    let cover = track.album.covers
        .iter()
        .max_by_key(|c| size_rank(c.size))
        .ok_or_else(|| Error::failed_precondition("Album has no cover"))?;
    let cover_id = cover.id.to_string();
    {
        let cache = ctx.covers.read().map_err(|_| Error::unavailable("Cover Cache Check"))?;
        if let Some(cover) = cache.get(&cover_id) {
            return Ok(Arc::clone(cover));
        }
    }
    download_cover(ctx, &cover_id).await?;
    let cache = ctx.covers.read().map_err(|_| Error::unavailable("Cover Cache Read"))?;
    Ok(Arc::clone(cache.get(&cover_id).unwrap()))
}   

pub async fn download_cover(ctx: &mut DLContext, id: &String) -> Result<(), Error> {
    let request = Request::builder()
        .method(&Method::GET)
        .uri(format!("{}{}", IMAGE_URL, id))
        .header(ACCEPT, HeaderValue::from_static("image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8"))
        .body(Bytes::new())?;
    let body = ctx.session.http_client().request_body(request).await?;
    let cover = Arc::new(Cover {
        data: body.to_vec(),
        mime: infer::get(&body)
            .map(|t| MimeType::from_str(t.mime_type()))
            .unwrap_or(MimeType::Jpeg),
    });
    let mut cache = ctx.covers.write().map_err(|_| Error::unavailable("Cover Cache Write"))?;
    cache.insert(id.to_owned(), cover);
    Ok(())
}
