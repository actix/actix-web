use error::Result;
use header::{self, EntityTag};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware;

use std::marker::PhantomData;

/// Trait for generating ETags from `(request, response)` pairs.
pub trait Generate<S> {
    fn generate(
        &mut self, req: &HttpRequest<S>, res: &HttpResponse,
    ) -> Option<EntityTag>;
}

/// The default generator computes an ETag as a cryptographic hash of the bytes of the
/// response body.
pub struct DefaultGenerator {
    hashstate: ::sha1::Sha1,
}
impl DefaultGenerator {
    /// Create a new instance.
    pub fn new() -> Self {
        DefaultGenerator {
            hashstate: ::sha1::Sha1::new(),
        }
    }
}
impl<S> Generate<S> for DefaultGenerator {
    fn generate(
        &mut self, req: &HttpRequest<S>, res: &HttpResponse,
    ) -> Option<EntityTag> {
        use http::{Method, StatusCode};
        use Body;

        if !(*req.method() == Method::GET && res.status() == StatusCode::OK) {
            return None;
        }

        match res.body() {
            Body::Binary(b) => {
                self.hashstate.reset();
                self.hashstate.update(b.as_ref());
                let tag = self.hashstate.digest().to_string();
                Some(EntityTag::strong(tag))
            }
            _ => None,
        }
    }
}

pub struct Generator<S: 'static, G: Generate<S> + 'static> {
    generator: G,
    _phantom: PhantomData<S>,
}
impl<S: 'static, G: Generate<S> + 'static> Generator<S, G> {
    /// Create a new middleware struct for ETag generation.
    pub fn new(generator: G) -> Self {
        Generator {
            generator,
            _phantom: PhantomData,
        }
    }
}
impl<S: 'static, G: Generate<S> + 'static> middleware::Middleware<S>
    for Generator<S, G>
{
    fn response(
        &mut self, req: &mut HttpRequest<S>, mut res: HttpResponse,
    ) -> Result<middleware::Response> {
        use header;
        // If response already has an ETag, do nothing
        if res.headers().contains_key(header::ETAG) {
            return Ok(middleware::Response::Done(res));
        }
        if let Some(etag) = self.generator.generate(&req, &res) {
            etag.to_string()
                .parse::<header::HeaderValue>()
                .map(|v| {
                    res.headers_mut().insert(header::ETAG, v);
                })
                .unwrap_or(());
        }
        Ok(middleware::Response::Done(res))
    }
}

// If-None-Match / 304 Not Modified support
pub struct NotModified;

impl<S> middleware::Middleware<S> for NotModified {
    fn response(
        &mut self, req: &mut HttpRequest<S>, res: HttpResponse,
    ) -> Result<middleware::Response> {
        use http::{Method, StatusCode};

        if !(*req.method() == Method::GET && res.status() == StatusCode::OK) {
            return Ok(middleware::Response::Done(res));
        }
        let etag = match response_etag(&res) {
            Some(v) => v,
            None => return Ok(middleware::Response::Done(res)),
        };

        if !none_match(&etag, req) {
            let mut not_modified =
                HttpResponse::NotModified().set(header::ETag(etag)).finish();

            // RFC 7232 requires copying over these headers:
            copy_header(header::CACHE_CONTROL, &res, &mut not_modified);
            copy_header(header::CONTENT_LOCATION, &res, &mut not_modified);
            copy_header(header::DATE, &res, &mut not_modified);
            copy_header(header::EXPIRES, &res, &mut not_modified);
            copy_header(header::VARY, &res, &mut not_modified);

            return Ok(middleware::Response::Done(not_modified));
        }
        Ok(middleware::Response::Done(res))
    }
}

#[inline]
fn response_etag(res: &HttpResponse) -> Option<EntityTag> {
    use std::str::FromStr;
    let e = res.headers().get(&header::ETAG)?.to_str().ok()?;
    Some(EntityTag::from_str(e).ok()?)
}
#[inline]
fn copy_header(h: header::HeaderName, src: &HttpResponse, dst: &mut HttpResponse) {
    if let Some(val) = src.headers().get(&h) {
        dst.headers_mut().insert(h, val.clone());
    }
}

// Returns true if `req` doesn't have an `If-None-Match` header matching `req`.
#[inline]
fn none_match<S>(etag: &EntityTag, req: &HttpRequest<S>) -> bool {
    use header::IfNoneMatch;
    use httpmessage::HttpMessage;
    match req.get_header::<IfNoneMatch>() {
        Some(IfNoneMatch::Items(ref items)) => {
            for item in items {
                if item.weak_eq(etag) {
                    return false;
                }
            }
            true
        }
        Some(IfNoneMatch::Any) => false,
        None => true,
    }
}
