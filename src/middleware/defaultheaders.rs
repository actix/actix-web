//! Default response headers
use http::{HeaderMap, HttpTryFrom};
use http::header::{HeaderName, HeaderValue, CONTENT_TYPE};

use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::{Response, Middleware};

/// `Middleware` for setting default response headers.
///
/// This middleware does not set header if response headers already contains it.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::*;
///
/// fn main() {
///     let app = Application::new()
///         .middleware(
///             middleware::DefaultHeaders::build()
///                 .header("X-Version", "0.2")
///                 .finish())
///         .resource("/test", |r| {
///              r.method(Method::GET).f(|_| httpcodes::HTTPOk);
///              r.method(Method::HEAD).f(|_| httpcodes::HTTPMethodNotAllowed);
///         })
///         .finish();
/// }
/// ```
pub struct DefaultHeaders{
    ct: bool,
    headers: HeaderMap,
}

impl DefaultHeaders {
    pub fn build() -> DefaultHeadersBuilder {
        DefaultHeadersBuilder{ct: false, headers: Some(HeaderMap::new())}
    }
}

impl<S> Middleware<S> for DefaultHeaders {

    fn response(&self, _: &mut HttpRequest<S>, mut resp: HttpResponse) -> Response {
        for (key, value) in self.headers.iter() {
            if !resp.headers().contains_key(key) {
                resp.headers_mut().insert(key, value.clone());
            }
        }
        // default content-type
        if self.ct && !resp.headers().contains_key(CONTENT_TYPE) {
            resp.headers_mut().insert(
                CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
        }
        Response::Done(resp)
    }
}

/// Structure that follows the builder pattern for building `DefaultHeaders` middleware.
#[derive(Debug)]
pub struct DefaultHeadersBuilder {
    ct: bool,
    headers: Option<HeaderMap>,
}

impl DefaultHeadersBuilder {

    /// Set a header.
    #[inline]
    #[cfg_attr(feature = "cargo-clippy", allow(match_wild_err_arm))]
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
        where HeaderName: HttpTryFrom<K>,
              HeaderValue: HttpTryFrom<V>
    {
        if let Some(ref mut headers) = self.headers {
            match HeaderName::try_from(key) {
                Ok(key) => {
                    match HeaderValue::try_from(value) {
                        Ok(value) => { headers.append(key, value); }
                        Err(_) => panic!("Can not create header value"),
                    }
                },
                Err(_) => panic!("Can not create header name"),
            };
        }
        self
    }

    /// Set *CONTENT-TYPE* header if response does not contain this header.
    pub fn content_type(&mut self) -> &mut Self {
        self.ct = true;
        self
    }

    /// Finishes building and returns the built `DefaultHeaders` middleware.
    pub fn finish(&mut self) -> DefaultHeaders {
        let headers = self.headers.take().expect("cannot reuse middleware builder");
        DefaultHeaders{ ct: self.ct, headers: headers }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::CONTENT_TYPE;

    #[test]
    fn test_default_headers() {
        let mw = DefaultHeaders::build()
            .header(CONTENT_TYPE, "0001")
            .finish();

        let mut req = HttpRequest::default();

        let resp = HttpResponse::Ok().finish().unwrap();
        let resp = match mw.response(&mut req, resp) {
            Response::Done(resp) => resp,
            _ => panic!(),
        };
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");

        let resp = HttpResponse::Ok().header(CONTENT_TYPE, "0002").finish().unwrap();
        let resp = match mw.response(&mut req, resp) {
            Response::Done(resp) => resp,
            _ => panic!(),
        };
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0002");
    }
}
