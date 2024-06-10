use std::{
    cell::{Ref, RefMut},
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::{error::HttpError, Response, ResponseHead};
use bytes::Bytes;
use futures_core::Stream;
use serde::Serialize;

use crate::{
    body::{BodyStream, BoxBody, MessageBody},
    dev::Extensions,
    error::{Error, JsonPayloadError},
    http::{
        header::{self, HeaderName, TryIntoHeaderPair, TryIntoHeaderValue},
        ConnectionType, StatusCode,
    },
    BoxError, HttpRequest, HttpResponse, Responder,
};

/// An HTTP response builder.
///
/// This type can be used to construct an instance of `Response` through a builder-like pattern.
pub struct HttpResponseBuilder {
    res: Option<Response<BoxBody>>,
    error: Option<HttpError>,
}

impl HttpResponseBuilder {
    #[inline]
    /// Create response builder
    pub fn new(status: StatusCode) -> Self {
        Self {
            res: Some(Response::with_body(status, BoxBody::new(()))),
            error: None,
        }
    }

    /// Set HTTP status code of this response.
    #[inline]
    pub fn status(&mut self, status: StatusCode) -> &mut Self {
        if let Some(parts) = self.inner() {
            parts.status = status;
        }
        self
    }

    /// Insert a header, replacing any that were set with an equivalent field name.
    ///
    /// ```
    /// use actix_web::{HttpResponse, http::header};
    ///
    /// HttpResponse::Ok()
    ///     .insert_header(header::ContentType(mime::APPLICATION_JSON))
    ///     .insert_header(("X-TEST", "value"))
    ///     .finish();
    /// ```
    pub fn insert_header(&mut self, header: impl TryIntoHeaderPair) -> &mut Self {
        if let Some(parts) = self.inner() {
            match header.try_into_pair() {
                Ok((key, value)) => {
                    parts.headers.insert(key, value);
                }
                Err(err) => self.error = Some(err.into()),
            };
        }

        self
    }

    /// Append a header, keeping any that were set with an equivalent field name.
    ///
    /// ```
    /// use actix_web::{HttpResponse, http::header};
    ///
    /// HttpResponse::Ok()
    ///     .append_header(header::ContentType(mime::APPLICATION_JSON))
    ///     .append_header(("X-TEST", "value1"))
    ///     .append_header(("X-TEST", "value2"))
    ///     .finish();
    /// ```
    pub fn append_header(&mut self, header: impl TryIntoHeaderPair) -> &mut Self {
        if let Some(parts) = self.inner() {
            match header.try_into_pair() {
                Ok((key, value)) => parts.headers.append(key, value),
                Err(err) => self.error = Some(err.into()),
            };
        }

        self
    }

    /// Replaced with [`Self::insert_header()`].
    #[doc(hidden)]
    #[deprecated(
        since = "4.0.0",
        note = "Replaced with `insert_header((key, value))`. Will be removed in v5."
    )]
    pub fn set_header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: TryInto<HeaderName>,
        K::Error: Into<HttpError>,
        V: TryIntoHeaderValue,
    {
        if self.error.is_some() {
            return self;
        }

        match (key.try_into(), value.try_into_value()) {
            (Ok(name), Ok(value)) => return self.insert_header((name, value)),
            (Err(err), _) => self.error = Some(err.into()),
            (_, Err(err)) => self.error = Some(err.into()),
        }

        self
    }

    /// Replaced with [`Self::append_header()`].
    #[doc(hidden)]
    #[deprecated(
        since = "4.0.0",
        note = "Replaced with `append_header((key, value))`. Will be removed in v5."
    )]
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: TryInto<HeaderName>,
        K::Error: Into<HttpError>,
        V: TryIntoHeaderValue,
    {
        if self.error.is_some() {
            return self;
        }

        match (key.try_into(), value.try_into_value()) {
            (Ok(name), Ok(value)) => return self.append_header((name, value)),
            (Err(err), _) => self.error = Some(err.into()),
            (_, Err(err)) => self.error = Some(err.into()),
        }

        self
    }

    /// Set the custom reason for the response.
    #[inline]
    pub fn reason(&mut self, reason: &'static str) -> &mut Self {
        if let Some(parts) = self.inner() {
            parts.reason = Some(reason);
        }
        self
    }

    /// Set connection type to KeepAlive
    #[inline]
    pub fn keep_alive(&mut self) -> &mut Self {
        if let Some(parts) = self.inner() {
            parts.set_connection_type(ConnectionType::KeepAlive);
        }
        self
    }

    /// Set connection type to Upgrade
    #[inline]
    pub fn upgrade<V>(&mut self, value: V) -> &mut Self
    where
        V: TryIntoHeaderValue,
    {
        if let Some(parts) = self.inner() {
            parts.set_connection_type(ConnectionType::Upgrade);
        }

        if let Ok(value) = value.try_into_value() {
            self.insert_header((header::UPGRADE, value));
        }

        self
    }

    /// Force close connection, even if it is marked as keep-alive
    #[inline]
    pub fn force_close(&mut self) -> &mut Self {
        if let Some(parts) = self.inner() {
            parts.set_connection_type(ConnectionType::Close);
        }
        self
    }

    /// Disable chunked transfer encoding for HTTP/1.1 streaming responses.
    #[inline]
    pub fn no_chunking(&mut self, len: u64) -> &mut Self {
        let mut buf = itoa::Buffer::new();
        self.insert_header((header::CONTENT_LENGTH, buf.format(len)));

        if let Some(parts) = self.inner() {
            parts.no_chunking(true);
        }
        self
    }

    /// Set response content type.
    #[inline]
    pub fn content_type<V>(&mut self, value: V) -> &mut Self
    where
        V: TryIntoHeaderValue,
    {
        if let Some(parts) = self.inner() {
            match value.try_into_value() {
                Ok(value) => {
                    parts.headers.insert(header::CONTENT_TYPE, value);
                }
                Err(err) => self.error = Some(err.into()),
            };
        }
        self
    }

    /// Add a cookie to the response.
    ///
    /// To send a "removal" cookie, call [`.make_removal()`](cookie::Cookie::make_removal) on the
    /// given cookie. See [`HttpResponse::add_removal_cookie()`] to learn more.
    ///
    /// # Examples
    /// Send a new cookie:
    /// ```
    /// use actix_web::{HttpResponse, cookie::Cookie};
    ///
    /// let res = HttpResponse::Ok()
    ///     .cookie(
    ///         Cookie::build("name", "value")
    ///             .domain("www.rust-lang.org")
    ///             .path("/")
    ///             .secure(true)
    ///             .http_only(true)
    ///             .finish(),
    ///     )
    ///     .finish();
    /// ```
    ///
    /// Send a removal cookie:
    /// ```
    /// use actix_web::{HttpResponse, cookie::Cookie};
    ///
    /// // the name, domain and path match the cookie created in the previous example
    /// let mut cookie = Cookie::build("name", "value-does-not-matter")
    ///     .domain("www.rust-lang.org")
    ///     .path("/")
    ///     .finish();
    /// cookie.make_removal();
    ///
    /// let res = HttpResponse::Ok()
    ///     .cookie(cookie)
    ///     .finish();
    /// ```
    #[cfg(feature = "cookies")]
    pub fn cookie(&mut self, cookie: cookie::Cookie<'_>) -> &mut Self {
        match cookie.to_string().try_into_value() {
            Ok(hdr_val) => self.append_header((header::SET_COOKIE, hdr_val)),
            Err(err) => {
                self.error = Some(err.into());
                self
            }
        }
    }

    /// Returns a reference to the response-local data/extensions container.
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        self.res
            .as_ref()
            .expect("cannot reuse response builder")
            .extensions()
    }

    /// Returns a mutable reference to the response-local data/extensions container.
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<'_, Extensions> {
        self.res
            .as_mut()
            .expect("cannot reuse response builder")
            .extensions_mut()
    }

    /// Set a body and build the `HttpResponse`.
    ///
    /// Unlike [`message_body`](Self::message_body), errors are converted into error
    /// responses immediately.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    pub fn body<B>(&mut self, body: B) -> HttpResponse<BoxBody>
    where
        B: MessageBody + 'static,
    {
        match self.message_body(body) {
            Ok(res) => res.map_into_boxed_body(),
            Err(err) => HttpResponse::from_error(err),
        }
    }

    /// Set a body and build the `HttpResponse`.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    pub fn message_body<B>(&mut self, body: B) -> Result<HttpResponse<B>, Error> {
        if let Some(err) = self.error.take() {
            return Err(err.into());
        }

        let res = self
            .res
            .take()
            .expect("cannot reuse response builder")
            .set_body(body);

        Ok(HttpResponse::from(res))
    }

    /// Set a streaming body and build the `HttpResponse`.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    #[inline]
    pub fn streaming<S, E>(&mut self, stream: S) -> HttpResponse
    where
        S: Stream<Item = Result<Bytes, E>> + 'static,
        E: Into<BoxError> + 'static,
    {
        self.body(BodyStream::new(stream))
    }

    /// Set a JSON body and build the `HttpResponse`.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    pub fn json(&mut self, value: impl Serialize) -> HttpResponse {
        match serde_json::to_string(&value) {
            Ok(body) => {
                let contains = if let Some(parts) = self.inner() {
                    parts.headers.contains_key(header::CONTENT_TYPE)
                } else {
                    true
                };

                if !contains {
                    self.insert_header((header::CONTENT_TYPE, mime::APPLICATION_JSON));
                }

                self.body(body)
            }
            Err(err) => HttpResponse::from_error(JsonPayloadError::Serialize(err)),
        }
    }

    /// Set an empty body and build the `HttpResponse`.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    #[inline]
    pub fn finish(&mut self) -> HttpResponse {
        self.body(())
    }

    /// This method construct new `HttpResponseBuilder`
    pub fn take(&mut self) -> Self {
        Self {
            res: self.res.take(),
            error: self.error.take(),
        }
    }

    fn inner(&mut self) -> Option<&mut ResponseHead> {
        if self.error.is_some() {
            return None;
        }

        self.res.as_mut().map(Response::head_mut)
    }
}

impl From<HttpResponseBuilder> for HttpResponse {
    fn from(mut builder: HttpResponseBuilder) -> Self {
        builder.finish()
    }
}

impl From<HttpResponseBuilder> for Response<BoxBody> {
    fn from(mut builder: HttpResponseBuilder) -> Self {
        builder.finish().into()
    }
}

impl Future for HttpResponseBuilder {
    type Output = Result<HttpResponse, Error>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(self.finish()))
    }
}

impl Responder for HttpResponseBuilder {
    type Body = BoxBody;

    #[inline]
    fn respond_to(mut self, _: &HttpRequest) -> HttpResponse<Self::Body> {
        self.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        body,
        http::header::{HeaderValue, CONTENT_TYPE},
        test::assert_body_eq,
    };

    #[test]
    fn test_basic_builder() {
        let resp = HttpResponse::Ok()
            .insert_header(("X-TEST", "value"))
            .finish();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_upgrade() {
        let resp = HttpResponseBuilder::new(StatusCode::OK)
            .upgrade("websocket")
            .finish();
        assert!(resp.upgrade());
        assert_eq!(
            resp.headers().get(header::UPGRADE).unwrap(),
            HeaderValue::from_static("websocket")
        );
    }

    #[test]
    fn test_force_close() {
        let resp = HttpResponseBuilder::new(StatusCode::OK)
            .force_close()
            .finish();
        assert!(!resp.keep_alive())
    }

    #[test]
    fn test_content_type() {
        let resp = HttpResponseBuilder::new(StatusCode::OK)
            .content_type("text/plain")
            .body(Bytes::new());
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "text/plain")
    }

    #[actix_rt::test]
    async fn test_json() {
        let res = HttpResponse::Ok().json(vec!["v1", "v2", "v3"]);
        let ct = res.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("application/json"));
        assert_body_eq!(res, br#"["v1","v2","v3"]"#);

        let res = HttpResponse::Ok().json(["v1", "v2", "v3"]);
        let ct = res.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("application/json"));
        assert_body_eq!(res, br#"["v1","v2","v3"]"#);

        // content type override
        let res = HttpResponse::Ok()
            .insert_header((CONTENT_TYPE, "text/json"))
            .json(&vec!["v1", "v2", "v3"]);
        let ct = res.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("text/json"));
        assert_body_eq!(res, br#"["v1","v2","v3"]"#);
    }

    #[actix_rt::test]
    async fn test_serde_json_in_body() {
        let resp = HttpResponse::Ok()
            .body(serde_json::to_vec(&serde_json::json!({ "test-key": "test-value" })).unwrap());

        assert_eq!(
            body::to_bytes(resp.into_body()).await.unwrap().as_ref(),
            br#"{"test-key":"test-value"}"#
        );
    }

    #[test]
    fn response_builder_header_insert_kv() {
        let mut res = HttpResponse::Ok();
        res.insert_header(("Content-Type", "application/octet-stream"));
        let res = res.finish();

        assert_eq!(
            res.headers().get("Content-Type"),
            Some(&HeaderValue::from_static("application/octet-stream"))
        );
    }

    #[test]
    fn response_builder_header_insert_typed() {
        let mut res = HttpResponse::Ok();
        res.insert_header((header::CONTENT_TYPE, mime::APPLICATION_OCTET_STREAM));
        let res = res.finish();

        assert_eq!(
            res.headers().get("Content-Type"),
            Some(&HeaderValue::from_static("application/octet-stream"))
        );
    }

    #[test]
    fn response_builder_header_append_kv() {
        let mut res = HttpResponse::Ok();
        res.append_header(("Content-Type", "application/octet-stream"));
        res.append_header(("Content-Type", "application/json"));
        let res = res.finish();

        let headers: Vec<_> = res.headers().get_all("Content-Type").cloned().collect();
        assert_eq!(headers.len(), 2);
        assert!(headers.contains(&HeaderValue::from_static("application/octet-stream")));
        assert!(headers.contains(&HeaderValue::from_static("application/json")));
    }

    #[test]
    fn response_builder_header_append_typed() {
        let mut res = HttpResponse::Ok();
        res.append_header((header::CONTENT_TYPE, mime::APPLICATION_OCTET_STREAM));
        res.append_header((header::CONTENT_TYPE, mime::APPLICATION_JSON));
        let res = res.finish();

        let headers: Vec<_> = res.headers().get_all("Content-Type").cloned().collect();
        assert_eq!(headers.len(), 2);
        assert!(headers.contains(&HeaderValue::from_static("application/octet-stream")));
        assert!(headers.contains(&HeaderValue::from_static("application/json")));
    }
}
