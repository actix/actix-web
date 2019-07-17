//! Middleware for setting default response headers
use std::rc::Rc;

use actix_service::{Service, Transform};
use futures::future::{ok, FutureResult};
use futures::{Future, Poll};

use crate::http::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use crate::http::{HeaderMap, HttpTryFrom};
use crate::service::{ServiceRequest, ServiceResponse};
use crate::Error;

/// `Middleware` for setting default response headers.
///
/// This middleware does not set header if response headers already contains it.
///
/// ```rust
/// use actix_web::{web, http, middleware, App, HttpResponse};
///
/// fn main() {
///     let app = App::new()
///         .wrap(middleware::DefaultHeaders::new().header("X-Version", "0.2"))
///         .service(
///             web::resource("/test")
///                 .route(web::get().to(|| HttpResponse::Ok()))
///                 .route(web::method(http::Method::HEAD).to(|| HttpResponse::MethodNotAllowed()))
///         );
/// }
/// ```
#[derive(Clone)]
pub struct DefaultHeaders {
    inner: Rc<Inner>,
}

struct Inner {
    ct: bool,
    headers: HeaderMap,
}

impl Default for DefaultHeaders {
    fn default() -> Self {
        DefaultHeaders {
            inner: Rc::new(Inner {
                ct: false,
                headers: HeaderMap::new(),
            }),
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
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        HeaderValue: HttpTryFrom<V>,
    {
        #[allow(clippy::match_wild_err_arm)]
        match HeaderName::try_from(key) {
            Ok(key) => match HeaderValue::try_from(value) {
                Ok(value) => {
                    Rc::get_mut(&mut self.inner)
                        .expect("Multiple copies exist")
                        .headers
                        .append(key, value);
                }
                Err(_) => panic!("Can not create header value"),
            },
            Err(_) => panic!("Can not create header name"),
        }
        self
    }

    /// Set *CONTENT-TYPE* header if response does not contain this header.
    pub fn content_type(mut self) -> Self {
        Rc::get_mut(&mut self.inner)
            .expect("Multiple copies exist")
            .ct = true;
        self
    }
}

impl<S, B> Transform<S> for DefaultHeaders
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = DefaultHeadersMiddleware<S>;
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(DefaultHeadersMiddleware {
            service,
            inner: self.inner.clone(),
        })
    }
}

pub struct DefaultHeadersMiddleware<S> {
    service: S,
    inner: Rc<Inner>,
}

impl<S, B> Service for DefaultHeadersMiddleware<S>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Box<dyn Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        let inner = self.inner.clone();

        Box::new(self.service.call(req).map(move |mut res| {
            // set response headers
            for (key, value) in inner.headers.iter() {
                if !res.headers().contains_key(key) {
                    res.headers_mut().insert(key.clone(), value.clone());
                }
            }
            // default content-type
            if inner.ct && !res.headers().contains_key(&CONTENT_TYPE) {
                res.headers_mut().insert(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/octet-stream"),
                );
            }

            res
        }))
    }
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;

    use super::*;
    use crate::dev::ServiceRequest;
    use crate::http::header::CONTENT_TYPE;
    use crate::test::{block_on, ok_service, TestRequest};
    use crate::HttpResponse;

    #[test]
    fn test_default_headers() {
        let mut mw = block_on(
            DefaultHeaders::new()
                .header(CONTENT_TYPE, "0001")
                .new_transform(ok_service()),
        )
        .unwrap();

        let req = TestRequest::default().to_srv_request();
        let resp = block_on(mw.call(req)).unwrap();
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");

        let req = TestRequest::default().to_srv_request();
        let srv = |req: ServiceRequest| {
            req.into_response(HttpResponse::Ok().header(CONTENT_TYPE, "0002").finish())
        };
        let mut mw = block_on(
            DefaultHeaders::new()
                .header(CONTENT_TYPE, "0001")
                .new_transform(srv.into_service()),
        )
        .unwrap();
        let resp = block_on(mw.call(req)).unwrap();
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0002");
    }

    #[test]
    fn test_content_type() {
        let srv = |req: ServiceRequest| req.into_response(HttpResponse::Ok().finish());
        let mut mw = block_on(
            DefaultHeaders::new()
                .content_type()
                .new_transform(srv.into_service()),
        )
        .unwrap();

        let req = TestRequest::default().to_srv_request();
        let resp = block_on(mw.call(req)).unwrap();
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            "application/octet-stream"
        );
    }
}
