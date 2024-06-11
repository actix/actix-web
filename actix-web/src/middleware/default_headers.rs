//! For middleware documentation, see [`DefaultHeaders`].

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_http::error::HttpError;
use actix_utils::future::{ready, Ready};
use futures_core::ready;
use pin_project_lite::pin_project;

use crate::{
    dev::{Service, Transform},
    http::header::{HeaderMap, HeaderName, HeaderValue, TryIntoHeaderPair, CONTENT_TYPE},
    service::{ServiceRequest, ServiceResponse},
    Error,
};

/// Middleware for setting default response headers.
///
/// Headers with the same key that are already set in a response will *not* be overwritten.
///
/// # Examples
/// ```
/// use actix_web::{web, http, middleware, App, HttpResponse};
///
/// let app = App::new()
///     .wrap(middleware::DefaultHeaders::new().add(("X-Version", "0.2")))
///     .service(
///         web::resource("/test")
///             .route(web::get().to(|| HttpResponse::Ok()))
///             .route(web::method(http::Method::HEAD).to(|| HttpResponse::MethodNotAllowed()))
///     );
/// ```
#[derive(Debug, Clone, Default)]
pub struct DefaultHeaders {
    inner: Rc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    headers: HeaderMap,
}

impl DefaultHeaders {
    /// Constructs an empty `DefaultHeaders` middleware.
    #[inline]
    pub fn new() -> DefaultHeaders {
        DefaultHeaders::default()
    }

    /// Adds a header to the default set.
    ///
    /// # Panics
    /// Panics when resolved header name or value is invalid.
    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, header: impl TryIntoHeaderPair) -> Self {
        // standard header terminology `insert` or `append` for this method would make the behavior
        // of this middleware less obvious since it only adds the headers if they are not present

        match header.try_into_pair() {
            Ok((key, value)) => Rc::get_mut(&mut self.inner)
                .expect("All default headers must be added before cloning.")
                .headers
                .append(key, value),
            Err(err) => panic!("Invalid header: {}", err.into()),
        }

        self
    }

    #[doc(hidden)]
    #[deprecated(
        since = "4.0.0",
        note = "Prefer `.add((key, value))`. Will be removed in v5."
    )]
    pub fn header<K, V>(self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        self.add((
            HeaderName::try_from(key)
                .map_err(Into::into)
                .expect("Invalid header name"),
            HeaderValue::try_from(value)
                .map_err(Into::into)
                .expect("Invalid header value"),
        ))
    }

    /// Adds a default *Content-Type* header if response does not contain one.
    ///
    /// Default is `application/octet-stream`.
    pub fn add_content_type(self) -> Self {
        #[allow(clippy::declare_interior_mutable_const)]
        const HV_MIME: HeaderValue = HeaderValue::from_static("application/octet-stream");
        self.add((CONTENT_TYPE, HV_MIME))
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
            inner: Rc::clone(&self.inner),
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

    actix_service::forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let inner = self.inner.clone();
        let fut = self.service.call(req);

        DefaultHeaderFuture {
            fut,
            inner,
            _body: PhantomData,
        }
    }
}

pin_project! {
    pub struct DefaultHeaderFuture<S: Service<ServiceRequest>, B> {
        #[pin]
        fut: S::Future,
        inner: Rc<Inner>,
        _body: PhantomData<B>,
    }
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

        Poll::Ready(Ok(res))
    }
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;
    use actix_utils::future::ok;

    use super::*;
    use crate::{
        test::{self, TestRequest},
        HttpResponse,
    };

    #[actix_rt::test]
    async fn adding_default_headers() {
        let mw = DefaultHeaders::new()
            .add(("X-TEST", "0001"))
            .add(("X-TEST-TWO", HeaderValue::from_static("123")))
            .new_transform(test::ok_service())
            .await
            .unwrap();

        let req = TestRequest::default().to_srv_request();
        let res = mw.call(req).await.unwrap();
        assert_eq!(res.headers().get("x-test").unwrap(), "0001");
        assert_eq!(res.headers().get("x-test-two").unwrap(), "123");
    }

    #[actix_rt::test]
    async fn no_override_existing() {
        let req = TestRequest::default().to_srv_request();
        let srv = |req: ServiceRequest| {
            ok(req.into_response(
                HttpResponse::Ok()
                    .insert_header((CONTENT_TYPE, "0002"))
                    .finish(),
            ))
        };
        let mw = DefaultHeaders::new()
            .add((CONTENT_TYPE, "0001"))
            .new_transform(srv.into_service())
            .await
            .unwrap();
        let resp = mw.call(req).await.unwrap();
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0002");
    }

    #[actix_rt::test]
    async fn adding_content_type() {
        let mw = DefaultHeaders::new()
            .add_content_type()
            .new_transform(test::ok_service())
            .await
            .unwrap();

        let req = TestRequest::default().to_srv_request();
        let resp = mw.call(req).await.unwrap();
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            "application/octet-stream"
        );
    }

    #[test]
    #[should_panic]
    fn invalid_header_name() {
        DefaultHeaders::new().add((":", "hello"));
    }

    #[test]
    #[should_panic]
    fn invalid_header_value() {
        DefaultHeaders::new().add(("x-test", "\n"));
    }
}
