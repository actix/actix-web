use std::fmt;

use actix_http::{
    body::Body,
    error::InternalError,
    http::{header::IntoHeaderPair, Error as HttpError, HeaderMap, StatusCode},
};
use bytes::{Bytes, BytesMut};

use crate::{Error, HttpRequest, HttpResponse, HttpResponseBuilder};

/// Trait implemented by types that can be converted to an HTTP response.
///
/// Any types that implement this trait can be used in the return type of a handler.
pub trait Responder {
    /// Convert self to `HttpResponse`.
    fn respond_to(self, req: &HttpRequest) -> HttpResponse;

    /// Override a status code for a Responder.
    ///
    /// ```
    /// use actix_web::{http::StatusCode, HttpRequest, Responder};
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     "Welcome!".with_status(StatusCode::OK)
    /// }
    /// ```
    fn with_status(self, status: StatusCode) -> CustomResponder<Self>
    where
        Self: Sized,
    {
        CustomResponder::new(self).with_status(status)
    }

    /// Insert header to the final response.
    ///
    /// Overrides other headers with the same name.
    ///
    /// ```
    /// use actix_web::{web, HttpRequest, Responder};
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyObj {
    ///     name: String,
    /// }
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     web::Json(MyObj { name: "Name".to_owned() })
    ///         .with_header(("x-version", "1.2.3"))
    /// }
    /// ```
    fn with_header<H>(self, header: H) -> CustomResponder<Self>
    where
        Self: Sized,
        H: IntoHeaderPair,
    {
        CustomResponder::new(self).with_header(header)
    }
}

impl Responder for HttpResponse {
    #[inline]
    fn respond_to(self, _: &HttpRequest) -> HttpResponse {
        self
    }
}

impl Responder for actix_http::Response<Body> {
    #[inline]
    fn respond_to(self, _: &HttpRequest) -> HttpResponse {
        HttpResponse::from(self)
    }
}

impl Responder for HttpResponseBuilder {
    #[inline]
    fn respond_to(mut self, _: &HttpRequest) -> HttpResponse {
        self.finish()
    }
}

impl Responder for actix_http::ResponseBuilder {
    #[inline]
    fn respond_to(mut self, _: &HttpRequest) -> HttpResponse {
        HttpResponse::from(self.finish())
    }
}

impl<T: Responder> Responder for Option<T> {
    fn respond_to(self, req: &HttpRequest) -> HttpResponse {
        match self {
            Some(val) => val.respond_to(req),
            None => HttpResponse::new(StatusCode::NOT_FOUND),
        }
    }
}

impl<T, E> Responder for Result<T, E>
where
    T: Responder,
    E: Into<Error>,
{
    fn respond_to(self, req: &HttpRequest) -> HttpResponse {
        match self {
            Ok(val) => val.respond_to(req),
            Err(e) => HttpResponse::from_error(e.into()),
        }
    }
}

impl<T: Responder> Responder for (T, StatusCode) {
    fn respond_to(self, req: &HttpRequest) -> HttpResponse {
        let mut res = self.0.respond_to(req);
        *res.status_mut() = self.1;
        res
    }
}

impl Responder for &'static str {
    fn respond_to(self, _: &HttpRequest) -> HttpResponse {
        HttpResponse::Ok()
            .content_type(mime::TEXT_PLAIN_UTF_8)
            .body(self)
    }
}

impl Responder for &'static [u8] {
    fn respond_to(self, _: &HttpRequest) -> HttpResponse {
        HttpResponse::Ok()
            .content_type(mime::APPLICATION_OCTET_STREAM)
            .body(self)
    }
}

impl Responder for String {
    fn respond_to(self, _: &HttpRequest) -> HttpResponse {
        HttpResponse::Ok()
            .content_type(mime::TEXT_PLAIN_UTF_8)
            .body(self)
    }
}

impl<'a> Responder for &'a String {
    fn respond_to(self, _: &HttpRequest) -> HttpResponse {
        HttpResponse::Ok()
            .content_type(mime::TEXT_PLAIN_UTF_8)
            .body(self)
    }
}

impl Responder for Bytes {
    fn respond_to(self, _: &HttpRequest) -> HttpResponse {
        HttpResponse::Ok()
            .content_type(mime::APPLICATION_OCTET_STREAM)
            .body(self)
    }
}

impl Responder for BytesMut {
    fn respond_to(self, _: &HttpRequest) -> HttpResponse {
        HttpResponse::Ok()
            .content_type(mime::APPLICATION_OCTET_STREAM)
            .body(self)
    }
}

/// Allows overriding status code and headers for a responder.
pub struct CustomResponder<T> {
    responder: T,
    status: Option<StatusCode>,
    headers: Result<HeaderMap, HttpError>,
}

impl<T: Responder> CustomResponder<T> {
    fn new(responder: T) -> Self {
        CustomResponder {
            responder,
            status: None,
            headers: Ok(HeaderMap::new()),
        }
    }

    /// Override a status code for the Responder's response.
    ///
    /// ```
    /// use actix_web::{HttpRequest, Responder, http::StatusCode};
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     "Welcome!".with_status(StatusCode::OK)
    /// }
    /// ```
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status = Some(status);
        self
    }

    /// Insert header to the final response.
    ///
    /// Overrides other headers with the same name.
    ///
    /// ```
    /// use actix_web::{web, HttpRequest, Responder};
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyObj {
    ///     name: String,
    /// }
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     web::Json(MyObj { name: "Name".to_string() })
    ///         .with_header(("x-version", "1.2.3"))
    ///         .with_header(("x-version", "1.2.3"))
    /// }
    /// ```
    pub fn with_header<H>(mut self, header: H) -> Self
    where
        H: IntoHeaderPair,
    {
        if let Ok(ref mut headers) = self.headers {
            match header.try_into_header_pair() {
                Ok((key, value)) => headers.append(key, value),
                Err(e) => self.headers = Err(e.into()),
            };
        }

        self
    }
}

impl<T: Responder> Responder for CustomResponder<T> {
    fn respond_to(self, req: &HttpRequest) -> HttpResponse {
        let headers = match self.headers {
            Ok(headers) => headers,
            Err(err) => return HttpResponse::from_error(Error::from(err)),
        };

        let mut res = self.responder.respond_to(req);

        if let Some(status) = self.status {
            *res.status_mut() = status;
        }

        for (k, v) in headers {
            // TODO: before v4, decide if this should be append instead
            res.headers_mut().insert(k, v);
        }

        res
    }
}

impl<T> Responder for InternalError<T>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    fn respond_to(self, _: &HttpRequest) -> HttpResponse {
        HttpResponse::from_error(self.into())
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
    use crate::{error, web, App};

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

        let resp = "test".respond_to(&req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp = b"test".respond_to(&req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        let resp = "test".to_string().respond_to(&req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp = (&"test".to_string()).respond_to(&req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp = Bytes::from_static(b"test").respond_to(&req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        let resp = BytesMut::from(b"test".as_ref()).respond_to(&req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        // InternalError
        let resp = error::InternalError::new("err", StatusCode::BAD_REQUEST).respond_to(&req);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn test_result_responder() {
        let req = TestRequest::default().to_http_request();

        // Result<I, E>
        let resp = Ok::<_, Error>("test".to_string()).respond_to(&req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let res = Err::<String, _>(error::InternalError::new("err", StatusCode::BAD_REQUEST))
            .respond_to(&req);

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn test_custom_responder() {
        let req = TestRequest::default().to_http_request();
        let res = "test"
            .to_string()
            .with_status(StatusCode::BAD_REQUEST)
            .respond_to(&req);

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.body().bin_ref(), b"test");

        let res = "test"
            .to_string()
            .with_header(("content-type", "json"))
            .respond_to(&req);

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
        let res = ("test".to_string(), StatusCode::BAD_REQUEST).respond_to(&req);
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.body().bin_ref(), b"test");

        let req = TestRequest::default().to_http_request();
        let res = ("test".to_string(), StatusCode::OK)
            .with_header((CONTENT_TYPE, mime::APPLICATION_JSON))
            .respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.body().bin_ref(), b"test");
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/json")
        );
    }
}
