//! Middleware for setting default response headers
use std::convert::TryFrom;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use actix_service::{Service, Transform};
use futures_util::future::{ready, Ready};
use futures_util::ready;

use crate::http::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use crate::http::{Error as HttpError, HeaderMap};
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
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
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

impl<S, B> Transform<S, ServiceRequest> for DefaultHeaders
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = DefaultHeadersMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(DefaultHeadersMiddleware {
            service,
            inner: self.inner.clone(),
        }))
    }
}

pub struct DefaultHeadersMiddleware<S> {
    service: S,
    inner: Rc<Inner>,
}

impl<S, B> Service<ServiceRequest> for DefaultHeadersMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = DefaultHeaderFuture<S, B>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        let inner = self.inner.clone();
        let fut = self.service.call(req);

        DefaultHeaderFuture {
            fut,
            inner,
            _body: PhantomData,
        }
    }
}

#[pin_project::pin_project]
pub struct DefaultHeaderFuture<S: Service<ServiceRequest>, B> {
    #[pin]
    fut: S::Future,
    inner: Rc<Inner>,
    _body: PhantomData<B>,
}

impl<S, B> Future for DefaultHeaderFuture<S, B>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Output = <S::Future as Future>::Output;

    #[allow(clippy::borrow_interior_mutable_const)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let mut res = ready!(this.fut.poll(cx))?;
        // set response headers
        for (key, value) in this.inner.headers.iter() {
            if !res.headers().contains_key(key) {
                res.headers_mut().insert(key.clone(), value.clone());
            }
        }
        // default content-type
        if this.inner.ct && !res.headers().contains_key(&CONTENT_TYPE) {
            res.headers_mut().insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
        }
        Poll::Ready(Ok(res))
    }
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;
    use futures_util::future::ok;

    use super::*;
    use crate::dev::ServiceRequest;
    use crate::http::header::CONTENT_TYPE;
    use crate::test::{ok_service, TestRequest};
    use crate::HttpResponse;

    #[actix_rt::test]
    async fn test_default_headers() {
        let mut mw = DefaultHeaders::new()
            .header(CONTENT_TYPE, "0001")
            .new_transform(ok_service())
            .await
            .unwrap();

        let req = TestRequest::default().to_srv_request();
        let resp = mw.call(req).await.unwrap();
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");

        let req = TestRequest::default().to_srv_request();
        let srv = |req: ServiceRequest| {
            ok(req
                .into_response(HttpResponse::Ok().header(CONTENT_TYPE, "0002").finish()))
        };
        let mut mw = DefaultHeaders::new()
            .header(CONTENT_TYPE, "0001")
            .new_transform(srv.into_service())
            .await
            .unwrap();
        let resp = mw.call(req).await.unwrap();
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0002");
    }

    #[actix_rt::test]
    async fn test_content_type() {
        let srv =
            |req: ServiceRequest| ok(req.into_response(HttpResponse::Ok().finish()));
        let mut mw = DefaultHeaders::new()
            .content_type()
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let req = TestRequest::default().to_srv_request();
        let resp = mw.call(req).await.unwrap();
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            "application/octet-stream"
        );
    }
}
