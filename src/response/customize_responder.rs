use actix_http::{
    body::{EitherBody, MessageBody},
    error::HttpError,
    header::HeaderMap,
    header::TryIntoHeaderPair,
    StatusCode,
};

use crate::{BoxError, HttpRequest, HttpResponse, Responder};

/// Allows overriding status code and headers for a responder.
pub struct CustomizeResponder<R> {
    inner: CustomizeResponderInner<R>,
    error: Option<HttpError>,
}

struct CustomizeResponderInner<R> {
    responder: R,
    status: Option<StatusCode>,
    override_headers: HeaderMap,
    append_headers: HeaderMap,
}

impl<R: Responder> CustomizeResponder<R> {
    pub(crate) fn new(responder: R) -> Self {
        CustomizeResponder {
            inner: CustomizeResponderInner {
                responder,
                status: None,
                override_headers: HeaderMap::new(),
                append_headers: HeaderMap::new(),
            },
            error: None,
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
        if let Some(inner) = self.inner() {
            inner.status = Some(status);
        }

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
    pub fn insert_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        if let Some(inner) = self.inner() {
            match header.try_into_header_pair() {
                Ok((key, value)) => {
                    inner.override_headers.insert(key, value);
                }
                Err(err) => self.error = Some(err.into()),
            };
        }

        self
    }

    pub fn append_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        if let Some(inner) = self.inner() {
            match header.try_into_header_pair() {
                Ok((key, value)) => {
                    inner.append_headers.append(key, value);
                }
                Err(err) => self.error = Some(err.into()),
            };
        }

        self
    }

    #[doc(hidden)]
    #[deprecated(since = "4.0.0", note = "Renamed to `insert_header`.")]
    pub fn with_header(self, header: impl TryIntoHeaderPair) -> Self
    where
        Self: Sized,
    {
        self.insert_header(header)
    }

    fn inner(&mut self) -> Option<&mut CustomizeResponderInner<R>> {
        if self.error.is_some() {
            None
        } else {
            Some(&mut self.inner)
        }
    }
}

impl<T> Responder for CustomizeResponder<T>
where
    T: Responder,
    <T::Body as MessageBody>::Error: Into<BoxError>,
{
    type Body = EitherBody<T::Body>;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        if let Some(err) = self.error {
            return HttpResponse::from_error(err).map_into_right_body();
        }

        let mut res = self.inner.responder.respond_to(req);

        if let Some(status) = self.inner.status {
            *res.status_mut() = status;
        }

        for (k, v) in self.inner.override_headers {
            res.headers_mut().insert(k, v);
        }

        for (k, v) in self.inner.append_headers {
            res.headers_mut().append(k, v);
        }

        res.map_into_left_body()
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use actix_http::body::to_bytes;

    use super::*;
    use crate::{
        http::{
            header::{HeaderValue, CONTENT_TYPE},
            StatusCode,
        },
        test::TestRequest,
    };

    #[actix_rt::test]
    async fn customize_responder() {
        let req = TestRequest::default().to_http_request();
        let res = "test"
            .to_string()
            .customize()
            .with_status(StatusCode::BAD_REQUEST)
            .respond_to(&req);

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = "test"
            .to_string()
            .customize()
            .insert_header(("content-type", "json"))
            .respond_to(&req);

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("json")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );
    }

    #[actix_rt::test]
    async fn tuple_responder_with_status_code() {
        let req = TestRequest::default().to_http_request();
        let res = ("test".to_string(), StatusCode::BAD_REQUEST).respond_to(&req);
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let req = TestRequest::default().to_http_request();
        let res = ("test".to_string(), StatusCode::OK)
            .customize()
            .insert_header((CONTENT_TYPE, mime::APPLICATION_JSON))
            .respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/json")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );
    }
}
