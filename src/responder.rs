use std::convert::TryFrom;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_http::error::InternalError;
use actix_http::http::{
    header::IntoHeaderValue, Error as HttpError, HeaderMap, HeaderName, StatusCode,
};
use actix_http::{Error, Response, ResponseBuilder};
use bytes::{Bytes, BytesMut};
use futures_util::future::{err, ok, Either as EitherFuture, Ready};
use futures_util::ready;
use pin_project::pin_project;

use crate::request::HttpRequest;

/// Trait implemented by types that can be converted to a http response.
///
/// Types that implement this trait can be used as the return type of a handler.
pub trait Responder {
    /// The associated error which can be returned.
    type Error: Into<Error>;

    /// The future response value.
    type Future: Future<Output = Result<Response, Self::Error>>;

    /// Convert itself to `AsyncResult` or `Error`.
    fn respond_to(self, req: &HttpRequest) -> Self::Future;

    /// Override a status code for a Responder.
    ///
    /// ```rust
    /// use actix_web::{HttpRequest, Responder, http::StatusCode};
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     "Welcome!".with_status(StatusCode::OK)
    /// }
    /// # fn main() {}
    /// ```
    fn with_status(self, status: StatusCode) -> CustomResponder<Self>
    where
        Self: Sized,
    {
        CustomResponder::new(self).with_status(status)
    }

    /// Add header to the Responder's response.
    ///
    /// ```rust
    /// use actix_web::{web, HttpRequest, Responder};
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyObj {
    ///     name: String,
    /// }
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     web::Json(
    ///         MyObj{name: "Name".to_string()}
    ///     )
    ///     .with_header("x-version", "1.2.3")
    /// }
    /// # fn main() {}
    /// ```
    fn with_header<K, V>(self, key: K, value: V) -> CustomResponder<Self>
    where
        Self: Sized,
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        CustomResponder::new(self).with_header(key, value)
    }
}

impl Responder for Response {
    type Error = Error;
    type Future = Ready<Result<Response, Error>>;

    #[inline]
    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(self)
    }
}

impl<T> Responder for Option<T>
where
    T: Responder,
{
    type Error = T::Error;
    type Future = EitherFuture<T::Future, Ready<Result<Response, T::Error>>>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match self {
            Some(t) => EitherFuture::Left(t.respond_to(req)),
            None => {
                EitherFuture::Right(ok(Response::build(StatusCode::NOT_FOUND).finish()))
            }
        }
    }
}

impl<T, E> Responder for Result<T, E>
where
    T: Responder,
    E: Into<Error>,
{
    type Error = Error;
    type Future = EitherFuture<
        ResponseFuture<T::Future, T::Error>,
        Ready<Result<Response, Error>>,
    >;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match self {
            Ok(val) => EitherFuture::Left(ResponseFuture::new(val.respond_to(req))),
            Err(e) => EitherFuture::Right(err(e.into())),
        }
    }
}

impl Responder for ResponseBuilder {
    type Error = Error;
    type Future = Ready<Result<Response, Error>>;

    #[inline]
    fn respond_to(mut self, _: &HttpRequest) -> Self::Future {
        ok(self.finish())
    }
}

impl<T> Responder for (T, StatusCode)
where
    T: Responder,
{
    type Error = T::Error;
    type Future = CustomResponderFut<T>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        CustomResponderFut {
            fut: self.0.respond_to(req),
            status: Some(self.1),
            headers: None,
        }
    }
}

impl Responder for &'static str {
    type Error = Error;
    type Future = Ready<Result<Response, Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self))
    }
}

impl Responder for &'static [u8] {
    type Error = Error;
    type Future = Ready<Result<Response, Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self))
    }
}

impl Responder for String {
    type Error = Error;
    type Future = Ready<Result<Response, Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self))
    }
}

impl<'a> Responder for &'a String {
    type Error = Error;
    type Future = Ready<Result<Response, Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self))
    }
}

impl Responder for Bytes {
    type Error = Error;
    type Future = Ready<Result<Response, Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self))
    }
}

impl Responder for BytesMut {
    type Error = Error;
    type Future = Ready<Result<Response, Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self))
    }
}

/// Allows to override status code and headers for a responder.
pub struct CustomResponder<T> {
    responder: T,
    status: Option<StatusCode>,
    headers: Option<HeaderMap>,
    error: Option<HttpError>,
}

impl<T: Responder> CustomResponder<T> {
    fn new(responder: T) -> Self {
        CustomResponder {
            responder,
            status: None,
            headers: None,
            error: None,
        }
    }

    /// Override a status code for the Responder's response.
    ///
    /// ```rust
    /// use actix_web::{HttpRequest, Responder, http::StatusCode};
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     "Welcome!".with_status(StatusCode::OK)
    /// }
    /// # fn main() {}
    /// ```
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status = Some(status);
        self
    }

    /// Add header to the Responder's response.
    ///
    /// ```rust
    /// use actix_web::{web, HttpRequest, Responder};
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyObj {
    ///     name: String,
    /// }
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     web::Json(
    ///         MyObj{name: "Name".to_string()}
    ///     )
    ///     .with_header("x-version", "1.2.3")
    /// }
    /// # fn main() {}
    /// ```
    pub fn with_header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        if self.headers.is_none() {
            self.headers = Some(HeaderMap::new());
        }

        match HeaderName::try_from(key) {
            Ok(key) => match value.try_into() {
                Ok(value) => {
                    self.headers.as_mut().unwrap().append(key, value);
                }
                Err(e) => self.error = Some(e.into()),
            },
            Err(e) => self.error = Some(e.into()),
        };
        self
    }
}

impl<T: Responder> Responder for CustomResponder<T> {
    type Error = T::Error;
    type Future = CustomResponderFut<T>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        CustomResponderFut {
            fut: self.responder.respond_to(req),
            status: self.status,
            headers: self.headers,
        }
    }
}

#[pin_project]
pub struct CustomResponderFut<T: Responder> {
    #[pin]
    fut: T::Future,
    status: Option<StatusCode>,
    headers: Option<HeaderMap>,
}

impl<T: Responder> Future for CustomResponderFut<T> {
    type Output = Result<Response, T::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let mut res = match ready!(this.fut.poll(cx)) {
            Ok(res) => res,
            Err(e) => return Poll::Ready(Err(e)),
        };
        if let Some(status) = this.status.take() {
            *res.status_mut() = status;
        }
        if let Some(ref headers) = this.headers {
            for (k, v) in headers {
                res.headers_mut().insert(k.clone(), v.clone());
            }
        }
        Poll::Ready(Ok(res))
    }
}

impl<T> Responder for InternalError<T>
where
    T: std::fmt::Debug + std::fmt::Display + 'static,
{
    type Error = Error;
    type Future = Ready<Result<Response, Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        let err: Error = self.into();
        ok(err.into())
    }
}

#[pin_project]
pub struct ResponseFuture<T, E> {
    #[pin]
    fut: T,
    _t: PhantomData<E>,
}

impl<T, E> ResponseFuture<T, E> {
    pub fn new(fut: T) -> Self {
        ResponseFuture {
            fut,
            _t: PhantomData,
        }
    }
}

impl<T, E> Future for ResponseFuture<T, E>
where
    T: Future<Output = Result<Response, E>>,
    E: Into<Error>,
{
    type Output = Result<Response, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(ready!(self.project().fut.poll(cx)).map_err(|e| e.into()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use actix_service::Service;
    use bytes::{Bytes, BytesMut};

    use super::*;
    use crate::dev::{Body, ResponseBody};
    use crate::http::{header::CONTENT_TYPE, HeaderValue, StatusCode};
    use crate::test::{init_service, TestRequest};
    use crate::{error, web, App, HttpResponse};

    #[actix_rt::test]
    async fn test_option_responder() {
        let mut srv = init_service(
            App::new()
                .service(
                    web::resource("/none").to(|| async { Option::<&'static str>::None }),
                )
                .service(web::resource("/some").to(|| async { Some("some") })),
        )
        .await;

        let req = TestRequest::with_uri("/none").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/some").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        match resp.response().body() {
            ResponseBody::Body(Body::Bytes(ref b)) => {
                let bytes = b.clone();
                assert_eq!(bytes, Bytes::from_static(b"some"));
            }
            _ => panic!(),
        }
    }

    pub(crate) trait BodyTest {
        fn bin_ref(&self) -> &[u8];
        fn body(&self) -> &Body;
    }

    impl BodyTest for ResponseBody<Body> {
        fn bin_ref(&self) -> &[u8] {
            match self {
                ResponseBody::Body(ref b) => match b {
                    Body::Bytes(ref bin) => &bin,
                    _ => panic!(),
                },
                ResponseBody::Other(ref b) => match b {
                    Body::Bytes(ref bin) => &bin,
                    _ => panic!(),
                },
            }
        }
        fn body(&self) -> &Body {
            match self {
                ResponseBody::Body(ref b) => b,
                ResponseBody::Other(ref b) => b,
            }
        }
    }

    #[actix_rt::test]
    async fn test_responder() {
        let req = TestRequest::default().to_http_request();

        let resp: HttpResponse = "test".respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse = b"test".respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        let resp: HttpResponse = "test".to_string().respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse = (&"test".to_string()).respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse =
            Bytes::from_static(b"test").respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        let resp: HttpResponse = BytesMut::from(b"test".as_ref())
            .respond_to(&req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        // InternalError
        let resp: HttpResponse =
            error::InternalError::new("err", StatusCode::BAD_REQUEST)
                .respond_to(&req)
                .await
                .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn test_result_responder() {
        let req = TestRequest::default().to_http_request();

        // Result<I, E>
        let resp: HttpResponse = Ok::<_, Error>("test".to_string())
            .respond_to(&req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let res =
            Err::<String, _>(error::InternalError::new("err", StatusCode::BAD_REQUEST))
                .respond_to(&req)
                .await;
        assert!(res.is_err());
    }

    #[actix_rt::test]
    async fn test_custom_responder() {
        let req = TestRequest::default().to_http_request();
        let res = "test"
            .to_string()
            .with_status(StatusCode::BAD_REQUEST)
            .respond_to(&req)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.body().bin_ref(), b"test");

        let res = "test"
            .to_string()
            .with_header("content-type", "json")
            .respond_to(&req)
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.body().bin_ref(), b"test");
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("json")
        );
    }

    #[actix_rt::test]
    async fn test_tuple_responder_with_status_code() {
        let req = TestRequest::default().to_http_request();
        let res = ("test".to_string(), StatusCode::BAD_REQUEST)
            .respond_to(&req)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.body().bin_ref(), b"test");

        let req = TestRequest::default().to_http_request();
        let res = ("test".to_string(), StatusCode::OK)
            .with_header("content-type", "json")
            .respond_to(&req)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.body().bin_ref(), b"test");
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("json")
        );
    }
}
