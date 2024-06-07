use std::borrow::Cow;

use actix_http::{
    body::{BoxBody, EitherBody, MessageBody},
    header::TryIntoHeaderPair,
    StatusCode,
};
use bytes::{Bytes, BytesMut};

use super::CustomizeResponder;
use crate::{Error, HttpRequest, HttpResponse};

/// Trait implemented by types that can be converted to an HTTP response.
///
/// Any types that implement this trait can be used in the return type of a handler. Since handlers
/// will only have one return type, it is idiomatic to use opaque return types `-> impl Responder`.
///
/// # Implementations
/// It is often not required to implement `Responder` for your own types due to a broad base of
/// built-in implementations:
/// - `HttpResponse` and `HttpResponseBuilder`
/// - `Option<R>` where `R: Responder`
/// - `Result<R, E>` where `R: Responder` and [`E: ResponseError`](crate::ResponseError)
/// - `(R, StatusCode)` where `R: Responder`
/// - `&'static str`, `String`, `&'_ String`, `Cow<'_, str>`, [`ByteString`](bytestring::ByteString)
/// - `&'static [u8]`, `Vec<u8>`, `Bytes`, `BytesMut`
/// - [`Json<T>`](crate::web::Json) and [`Form<T>`](crate::web::Form) where `T: Serialize`
/// - [`Either<L, R>`](crate::web::Either) where `L: Serialize` and `R: Serialize`
/// - [`CustomizeResponder<R>`]
/// - [`actix_files::NamedFile`](https://docs.rs/actix-files/latest/actix_files/struct.NamedFile.html)
/// - [Experimental responders from `actix-web-lab`](https://docs.rs/actix-web-lab/latest/actix_web_lab/respond/index.html)
/// - Third party integrations may also have implemented `Responder` where appropriate. For example,
///   HTML templating engines.
///
/// # Customizing Responder Output
/// Calling [`.customize()`](Responder::customize) on any responder type will wrap it in a
/// [`CustomizeResponder`] capable of overriding various parts of the response such as the status
/// code and header map.
pub trait Responder {
    type Body: MessageBody + 'static;

    /// Convert self to `HttpResponse`.
    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body>;

    /// Wraps responder to allow alteration of its response.
    ///
    /// See [`CustomizeResponder`] docs for more details on its capabilities.
    ///
    /// # Examples
    /// ```
    /// use actix_web::{Responder, http::StatusCode, test::TestRequest};
    ///
    /// let responder = "Hello world!"
    ///     .customize()
    ///     .with_status(StatusCode::BAD_REQUEST)
    ///     .insert_header(("x-hello", "world"));
    ///
    /// let request = TestRequest::default().to_http_request();
    /// let response = responder.respond_to(&request);
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    /// assert_eq!(response.headers().get("x-hello").unwrap(), "world");
    /// ```
    #[inline]
    fn customize(self) -> CustomizeResponder<Self>
    where
        Self: Sized,
    {
        CustomizeResponder::new(self)
    }

    #[doc(hidden)]
    #[deprecated(since = "4.0.0", note = "Prefer `.customize().with_status(header)`.")]
    fn with_status(self, status: StatusCode) -> CustomizeResponder<Self>
    where
        Self: Sized,
    {
        self.customize().with_status(status)
    }

    #[doc(hidden)]
    #[deprecated(since = "4.0.0", note = "Prefer `.customize().insert_header(header)`.")]
    fn with_header(self, header: impl TryIntoHeaderPair) -> CustomizeResponder<Self>
    where
        Self: Sized,
    {
        self.customize().insert_header(header)
    }
}

impl Responder for actix_http::Response<BoxBody> {
    type Body = BoxBody;

    #[inline]
    fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
        HttpResponse::from(self)
    }
}

impl Responder for actix_http::ResponseBuilder {
    type Body = BoxBody;

    #[inline]
    fn respond_to(mut self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        self.finish().map_into_boxed_body().respond_to(req)
    }
}

impl<R: Responder> Responder for Option<R> {
    type Body = EitherBody<R::Body>;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        match self {
            Some(val) => val.respond_to(req).map_into_left_body(),
            None => HttpResponse::new(StatusCode::NOT_FOUND).map_into_right_body(),
        }
    }
}

impl<R, E> Responder for Result<R, E>
where
    R: Responder,
    E: Into<Error>,
{
    type Body = EitherBody<R::Body>;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        match self {
            Ok(val) => val.respond_to(req).map_into_left_body(),
            Err(err) => HttpResponse::from_error(err.into()).map_into_right_body(),
        }
    }
}

impl<R: Responder> Responder for (R, StatusCode) {
    type Body = R::Body;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        let mut res = self.0.respond_to(req);
        *res.status_mut() = self.1;
        res
    }
}

macro_rules! impl_responder_by_forward_into_base_response {
    ($res:ty, $body:ty) => {
        impl Responder for $res {
            type Body = $body;

            fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
                let res: actix_http::Response<_> = self.into();
                res.into()
            }
        }
    };

    ($res:ty) => {
        impl_responder_by_forward_into_base_response!($res, $res);
    };
}

impl_responder_by_forward_into_base_response!(&'static [u8]);
impl_responder_by_forward_into_base_response!(Vec<u8>);
impl_responder_by_forward_into_base_response!(Bytes);
impl_responder_by_forward_into_base_response!(BytesMut);

impl_responder_by_forward_into_base_response!(&'static str);
impl_responder_by_forward_into_base_response!(String);
impl_responder_by_forward_into_base_response!(bytestring::ByteString);

macro_rules! impl_into_string_responder {
    ($res:ty) => {
        impl Responder for $res {
            type Body = String;

            fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
                let string: String = self.into();
                let res: actix_http::Response<_> = string.into();
                res.into()
            }
        }
    };
}

impl_into_string_responder!(&'_ String);
impl_into_string_responder!(Cow<'_, str>);

#[cfg(test)]
pub(crate) mod tests {
    use actix_http::body::to_bytes;
    use actix_service::Service;

    use super::*;
    use crate::{
        error,
        http::header::{HeaderValue, CONTENT_TYPE},
        test::{assert_body_eq, init_service, TestRequest},
        web, App,
    };

    #[actix_rt::test]
    async fn test_option_responder() {
        let srv = init_service(
            App::new()
                .service(web::resource("/none").to(|| async { Option::<&'static str>::None }))
                .service(web::resource("/some").to(|| async { Some("some") })),
        )
        .await;

        let req = TestRequest::with_uri("/none").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/some").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_body_eq!(resp, b"some");
    }

    #[actix_rt::test]
    async fn test_responder() {
        let req = TestRequest::default().to_http_request();

        let res = "test".respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = b"test".respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = "test".to_string().respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = (&"test".to_string()).respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let s = String::from("test");
        let res = Cow::Borrowed(s.as_str()).respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = Cow::<'_, str>::Owned(s).respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = Cow::Borrowed("test").respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = Bytes::from_static(b"test").respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = BytesMut::from(b"test".as_ref()).respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        // InternalError
        let res = error::InternalError::new("err", StatusCode::BAD_REQUEST).respond_to(&req);
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn test_result_responder() {
        let req = TestRequest::default().to_http_request();

        // Result<I, E>
        let resp = Ok::<_, Error>("test".to_string()).respond_to(&req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(
            to_bytes(resp.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = Err::<String, _>(error::InternalError::new("err", StatusCode::BAD_REQUEST))
            .respond_to(&req);

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }
}
