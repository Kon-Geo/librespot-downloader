use std::{cell::RefCell, collections::HashMap, sync::Arc};
use bytes::Bytes;
use http::{HeaderValue, Method, Request, header::ACCEPT};
use librespot::{core::{Error, http_client::HttpClient}, metadata::{Track, image}};
use lofty::picture::MimeType;
use crate::{config::IMAGE_URL};

pub struct Cover {
    pub data: Vec<u8>,
    pub mime: MimeType,
}

pub struct CoverCache {
    covers: RefCell<HashMap<String, Arc<Cover>>>,
}

impl CoverCache {
    pub fn new() -> Self {
        Self { covers: RefCell::new(HashMap::new()) }
    }

    fn get(&self, id: &str) -> Option<Arc<Cover>> {
        self.covers.borrow().get(id).cloned()
    }

    fn insert(&self, id: String, cover: Arc<Cover>) {
        self.covers.borrow_mut().insert(id, cover);
    }

    pub async fn get_cover(&mut self, http: &HttpClient, track: &Track) -> Result<Arc<Cover>, Error> {
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
            if let Some(cover) = self.get(&cover_id) {
                return Ok(cover);
            }
    
        }
        let cover = self.download_cover(http, &cover_id).await?;
        self.insert(cover_id.clone(), cover.clone());
        Ok(cover)
    }
    
    pub async fn download_cover(&self, http: &HttpClient, id: &String) -> Result<Arc<Cover>, Error> {
        let request = Request::builder()
            .method(&Method::GET)
            .uri(format!("{}{}", IMAGE_URL, id))
            .header(ACCEPT, HeaderValue::from_static("image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8"))
            .body(Bytes::new())?;
        let body = http.request_body(request).await?;
        let cover = Arc::new(Cover {
            data: body.to_vec(),
            mime: infer::get(&body)
                .map(|t| MimeType::from_str(t.mime_type()))
                .unwrap_or(MimeType::Jpeg),
        });
        Ok(cover)
    }
}
