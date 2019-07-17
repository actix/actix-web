use actix_http::error::InternalError;
use actix_http::http::{
    header::IntoHeaderValue, Error as HttpError, HeaderMap, HeaderName, HttpTryFrom,
    StatusCode,
};
use actix_http::{Error, Response, ResponseBuilder};
use bytes::{Bytes, BytesMut};
use futures::future::{err, ok, Either as EitherFuture, FutureResult};
use futures::{try_ready, Async, Future, IntoFuture, Poll};

use crate::request::HttpRequest;

/// Trait implemented by types that can be converted to a http response.
///
/// Types that implement this trait can be used as the return type of a handler.
pub trait Responder {
    /// The associated error which can be returned.
    type Error: Into<Error>;

    /// The future response value.
    type Future: IntoFuture<Item = Response, Error = Self::Error>;

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
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        CustomResponder::new(self).with_header(key, value)
    }
}

impl Responder for Response {
    type Error = Error;
    type Future = FutureResult<Response, Error>;

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
    type Future = EitherFuture<
        <T::Future as IntoFuture>::Future,
        FutureResult<Response, T::Error>,
    >;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match self {
            Some(t) => EitherFuture::A(t.respond_to(req).into_future()),
            None => EitherFuture::B(ok(Response::build(StatusCode::NOT_FOUND).finish())),
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
        ResponseFuture<<T::Future as IntoFuture>::Future>,
        FutureResult<Response, Error>,
    >;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match self {
            Ok(val) => {
                EitherFuture::A(ResponseFuture::new(val.respond_to(req).into_future()))
            }
            Err(e) => EitherFuture::B(err(e.into())),
        }
    }
}

impl Responder for ResponseBuilder {
    type Error = Error;
    type Future = FutureResult<Response, Error>;

    #[inline]
    fn respond_to(mut self, _: &HttpRequest) -> Self::Future {
        ok(self.finish())
    }
}

impl Responder for () {
    type Error = Error;
    type Future = FutureResult<Response, Error>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK).finish())
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
            fut: self.0.respond_to(req).into_future(),
            status: Some(self.1),
            headers: None,
        }
    }
}

impl Responder for &'static str {
    type Error = Error;
    type Future = FutureResult<Response, Error>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self))
    }
}

impl Responder for &'static [u8] {
    type Error = Error;
    type Future = FutureResult<Response, Error>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self))
    }
}

impl Responder for String {
    type Error = Error;
    type Future = FutureResult<Response, Error>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self))
    }
}

impl<'a> Responder for &'a String {
    type Error = Error;
    type Future = FutureResult<Response, Error>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self))
    }
}

impl Responder for Bytes {
    type Error = Error;
    type Future = FutureResult<Response, Error>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self))
    }
}

impl Responder for BytesMut {
    type Error = Error;
    type Future = FutureResult<Response, Error>;

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
        HeaderName: HttpTryFrom<K>,
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
            fut: self.responder.respond_to(req).into_future(),
            status: self.status,
            headers: self.headers,
        }
    }
}

pub struct CustomResponderFut<T: Responder> {
    fut: <T::Future as IntoFuture>::Future,
    status: Option<StatusCode>,
    headers: Option<HeaderMap>,
}

impl<T: Responder> Future for CustomResponderFut<T> {
    type Item = Response;
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut res = try_ready!(self.fut.poll());
        if let Some(status) = self.status {
            *res.status_mut() = status;
        }
        if let Some(ref headers) = self.headers {
            for (k, v) in headers {
                res.headers_mut().insert(k.clone(), v.clone());
            }
        }
        Ok(Async::Ready(res))
    }
}

/// Combines two different responder types into a single type
///
/// ```rust
/// # use futures::future::{ok, Future};
/// use actix_web::{Either, Error, HttpResponse};
///
/// type RegisterResult =
///     Either<HttpResponse, Box<dyn Future<Item = HttpResponse, Error = Error>>>;
///
/// fn index() -> RegisterResult {
///     if is_a_variant() {
///         // <- choose left variant
///         Either::A(HttpResponse::BadRequest().body("Bad data"))
///     } else {
///         Either::B(
///             // <- Right variant
///             Box::new(ok(HttpResponse::Ok()
///                 .content_type("text/html")
///                 .body("Hello!")))
///         )
///     }
/// }
/// # fn is_a_variant() -> bool { true }
/// # fn main() {}
/// ```
#[derive(Debug, PartialEq)]
pub enum Either<A, B> {
    /// First branch of the type
    A(A),
    /// Second branch of the type
    B(B),
}

impl<A, B> Responder for Either<A, B>
where
    A: Responder,
    B: Responder,
{
    type Error = Error;
    type Future = EitherResponder<
        <A::Future as IntoFuture>::Future,
        <B::Future as IntoFuture>::Future,
    >;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match self {
            Either::A(a) => EitherResponder::A(a.respond_to(req).into_future()),
            Either::B(b) => EitherResponder::B(b.respond_to(req).into_future()),
        }
    }
}

pub enum EitherResponder<A, B>
where
    A: Future<Item = Response>,
    A::Error: Into<Error>,
    B: Future<Item = Response>,
    B::Error: Into<Error>,
{
    A(A),
    B(B),
}

impl<A, B> Future for EitherResponder<A, B>
where
    A: Future<Item = Response>,
    A::Error: Into<Error>,
    B: Future<Item = Response>,
    B::Error: Into<Error>,
{
    type Item = Response;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self {
            EitherResponder::A(ref mut fut) => Ok(fut.poll().map_err(|e| e.into())?),
            EitherResponder::B(ref mut fut) => Ok(fut.poll().map_err(|e| e.into())?),
        }
    }
}

impl<I, E> Responder for Box<dyn Future<Item = I, Error = E>>
where
    I: Responder + 'static,
    E: Into<Error> + 'static,
{
    type Error = Error;
    type Future = Box<dyn Future<Item = Response, Error = Error>>;

    #[inline]
    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        let req = req.clone();
        Box::new(
            self.map_err(|e| e.into())
                .and_then(move |r| ResponseFuture(r.respond_to(&req).into_future())),
        )
    }
}

impl<T> Responder for InternalError<T>
where
    T: std::fmt::Debug + std::fmt::Display + 'static,
{
    type Error = Error;
    type Future = Result<Response, Error>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        let err: Error = self.into();
        Ok(err.into())
    }
}

pub struct ResponseFuture<T>(T);

impl<T> ResponseFuture<T> {
    pub fn new(fut: T) -> Self {
        ResponseFuture(fut)
    }
}

impl<T> Future for ResponseFuture<T>
where
    T: Future<Item = Response>,
    T::Error: Into<Error>,
{
    type Item = Response;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        Ok(self.0.poll().map_err(|e| e.into())?)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use actix_service::Service;
    use bytes::{Bytes, BytesMut};

    use super::*;
    use crate::dev::{Body, ResponseBody};
    use crate::http::{header::CONTENT_TYPE, HeaderValue, StatusCode};
    use crate::test::{block_on, init_service, TestRequest};
    use crate::{error, web, App, HttpResponse};

    #[test]
    fn test_option_responder() {
        let mut srv = init_service(
            App::new()
                .service(web::resource("/none").to(|| -> Option<&'static str> { None }))
                .service(web::resource("/some").to(|| Some("some"))),
        );

        let req = TestRequest::with_uri("/none").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/some").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        match resp.response().body() {
            ResponseBody::Body(Body::Bytes(ref b)) => {
                let bytes: Bytes = b.clone().into();
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

    #[test]
    fn test_responder() {
        let req = TestRequest::default().to_http_request();

        let resp: HttpResponse = block_on(().respond_to(&req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(*resp.body().body(), Body::Empty);

        let resp: HttpResponse = block_on("test".respond_to(&req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse = block_on(b"test".respond_to(&req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        let resp: HttpResponse = block_on("test".to_string().respond_to(&req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse =
            block_on((&"test".to_string()).respond_to(&req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse =
            block_on(Bytes::from_static(b"test").respond_to(&req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        let resp: HttpResponse =
            block_on(BytesMut::from(b"test".as_ref()).respond_to(&req)).unwrap();
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
                .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_result_responder() {
        let req = TestRequest::default().to_http_request();

        // Result<I, E>
        let resp: HttpResponse =
            block_on(Ok::<_, Error>("test".to_string()).respond_to(&req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let res = block_on(
            Err::<String, _>(error::InternalError::new("err", StatusCode::BAD_REQUEST))
                .respond_to(&req),
        );
        assert!(res.is_err());
    }

    #[test]
    fn test_custom_responder() {
        let req = TestRequest::default().to_http_request();
        let res = block_on(
            "test"
                .to_string()
                .with_status(StatusCode::BAD_REQUEST)
                .respond_to(&req),
        )
        .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.body().bin_ref(), b"test");

        let res = block_on(
            "test"
                .to_string()
                .with_header("content-type", "json")
                .respond_to(&req),
        )
        .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.body().bin_ref(), b"test");
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("json")
        );
    }

    #[test]
    fn test_tuple_responder_with_status_code() {
        let req = TestRequest::default().to_http_request();
        let res =
            block_on(("test".to_string(), StatusCode::BAD_REQUEST).respond_to(&req))
                .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.body().bin_ref(), b"test");

        let req = TestRequest::default().to_http_request();
        let res = block_on(
            ("test".to_string(), StatusCode::OK)
                .with_header("content-type", "json")
                .respond_to(&req),
        )
        .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.body().bin_ref(), b"test");
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("json")
        );
    }
}
