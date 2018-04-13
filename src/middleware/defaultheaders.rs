//! Default response headers
use http::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use http::{HeaderMap, HttpTryFrom};

use error::Result;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::{Middleware, Response};

/// `Middleware` for setting default response headers.
///
/// This middleware does not set header if response headers already contains it.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{http, middleware, App, HttpResponse};
///
/// fn main() {
///     let app = App::new()
///         .middleware(
///             middleware::DefaultHeaders::new()
///                 .header("X-Version", "0.2"))
///         .resource("/test", |r| {
///              r.method(http::Method::GET).f(|_| HttpResponse::Ok());
///              r.method(http::Method::HEAD).f(|_| HttpResponse::MethodNotAllowed());
///         })
///         .finish();
/// }
/// ```
pub struct DefaultHeaders {
    ct: bool,
    headers: HeaderMap,
}

impl Default for DefaultHeaders {
    fn default() -> Self {
        DefaultHeaders {
            ct: false,
            headers: HeaderMap::new(),
        }
    }
}

impl DefaultHeaders {
    /// Construct `DefaultHeaders` middleware.
    pub fn new() -> DefaultHeaders {
        DefaultHeaders::default()
    }

    /// Set a header.
    #[inline]
    #[cfg_attr(feature = "cargo-clippy", allow(match_wild_err_arm))]
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        HeaderValue: HttpTryFrom<V>,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match HeaderValue::try_from(value) {
                Ok(value) => {
                    self.headers.append(key, value);
                }
                Err(_) => panic!("Can not create header value"),
            },
            Err(_) => panic!("Can not create header name"),
        }
        self
    }

    /// Set *CONTENT-TYPE* header if response does not contain this header.
    pub fn content_type(mut self) -> Self {
        self.ct = true;
        self
    }
}

impl<S> Middleware<S> for DefaultHeaders {
    fn response(
        &self, _: &mut HttpRequest<S>, mut resp: HttpResponse
    ) -> Result<Response> {
        for (key, value) in self.headers.iter() {
            if !resp.headers().contains_key(key) {
                resp.headers_mut().insert(key, value.clone());
            }
        }
        // default content-type
        if self.ct && !resp.headers().contains_key(CONTENT_TYPE) {
            resp.headers_mut().insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
        }
        Ok(Response::Done(resp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::CONTENT_TYPE;

    #[test]
    fn test_default_headers() {
        let mw = DefaultHeaders::new().header(CONTENT_TYPE, "0001");

        let mut req = HttpRequest::default();

        let resp = HttpResponse::Ok().finish();
        let resp = match mw.response(&mut req, resp) {
            Ok(Response::Done(resp)) => resp,
            _ => panic!(),
        };
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");

        let resp = HttpResponse::Ok()
            .header(CONTENT_TYPE, "0002")
            .finish();
        let resp = match mw.response(&mut req, resp) {
            Ok(Response::Done(resp)) => resp,
            _ => panic!(),
        };
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0002");
    }
}
