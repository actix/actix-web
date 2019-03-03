//! Middleware for setting default response headers
use std::rc::Rc;

use actix_http::http::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use actix_http::http::{HeaderMap, HttpTryFrom};
use actix_service::{IntoNewTransform, Service, Transform};
use futures::{Async, Future, Poll};

use crate::middleware::MiddlewareFactory;
use crate::service::{ServiceRequest, ServiceResponse};

/// `Middleware` for setting default response headers.
///
/// This middleware does not set header if response headers already contains it.
///
/// ```rust
/// use actix_web::{web, http, middleware, App, HttpResponse};
///
/// fn main() {
///     let app = App::new()
///         .middleware(middleware::DefaultHeaders::new().header("X-Version", "0.2"))
///         .resource("/test", |r| {
///             r.route(web::get().to(|| HttpResponse::Ok()))
///              .route(web::method(http::Method::HEAD).to(|| HttpResponse::MethodNotAllowed()))
///         });
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

impl<S, State, B> IntoNewTransform<MiddlewareFactory<DefaultHeaders, S>, S>
    for DefaultHeaders
where
    S: Service<Request = ServiceRequest<State>, Response = ServiceResponse<B>>,
    S::Future: 'static,
{
    fn into_new_transform(self) -> MiddlewareFactory<DefaultHeaders, S> {
        MiddlewareFactory::new(self)
    }
}

impl<S, State, B> Transform<S> for DefaultHeaders
where
    S: Service<Request = ServiceRequest<State>, Response = ServiceResponse<B>>,
    S::Future: 'static,
{
    type Request = ServiceRequest<State>;
    type Response = ServiceResponse<B>;
    type Error = S::Error;
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: ServiceRequest<State>, srv: &mut S) -> Self::Future {
        let inner = self.inner.clone();

        Box::new(srv.call(req).map(move |mut res| {
            // set response headers
            for (key, value) in inner.headers.iter() {
                if !res.headers().contains_key(key) {
                    res.headers_mut().insert(key, value.clone());
                }
            }
            // default content-type
            if inner.ct && !res.headers().contains_key(CONTENT_TYPE) {
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
    use actix_http::http::header::CONTENT_TYPE;
    use actix_service::FnService;

    use super::*;
    use crate::test::TestServiceRequest;
    use crate::{HttpResponse, ServiceRequest};

    #[test]
    fn test_default_headers() {
        let mut rt = actix_rt::Runtime::new().unwrap();
        let mut mw = DefaultHeaders::new().header(CONTENT_TYPE, "0001");
        let mut srv = FnService::new(|req: ServiceRequest<_>| {
            req.into_response(HttpResponse::Ok().finish())
        });

        let req = TestServiceRequest::default().finish();
        let resp = rt.block_on(mw.call(req, &mut srv)).unwrap();
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");

        let req = TestServiceRequest::default().finish();
        let mut srv = FnService::new(|req: ServiceRequest<_>| {
            req.into_response(HttpResponse::Ok().header(CONTENT_TYPE, "0002").finish())
        });
        let resp = rt.block_on(mw.call(req, &mut srv)).unwrap();
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0002");
    }

    #[test]
    fn test_content_type() {
        let mut rt = actix_rt::Runtime::new().unwrap();
        let mut mw = DefaultHeaders::new().content_type();
        let mut srv = FnService::new(|req: ServiceRequest<_>| {
            req.into_response(HttpResponse::Ok().finish())
        });

        let req = TestServiceRequest::default().finish();
        let resp = rt.block_on(mw.call(req, &mut srv)).unwrap();
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            "application/octet-stream"
        );
    }
}
