//! HTTP response builder.

use std::{cell::RefCell, fmt, str};

use crate::{
    body::{EitherBody, MessageBody},
    error::{Error, HttpError},
    header::{self, TryIntoHeaderPair, TryIntoHeaderValue},
    responses::{BoxedResponseHead, ResponseHead},
    ConnectionType, Extensions, Response, StatusCode,
};

/// An HTTP response builder.
///
/// Used to construct an instance of `Response` using a builder pattern. Response builders are often
/// created using [`Response::build`].
///
/// # Examples
/// ```
/// use actix_http::{Response, ResponseBuilder, StatusCode, body, header};
///
/// # actix_rt::System::new().block_on(async {
/// let mut res: Response<_> = Response::build(StatusCode::OK)
///     .content_type(mime::APPLICATION_JSON)
///     .insert_header((header::SERVER, "my-app/1.0"))
///     .append_header((header::SET_COOKIE, "a=1"))
///     .append_header((header::SET_COOKIE, "b=2"))
///     .body("1234");
///
/// assert_eq!(res.status(), StatusCode::OK);
///
/// assert!(res.headers().contains_key("server"));
/// assert_eq!(res.headers().get_all("set-cookie").count(), 2);
///
/// assert_eq!(body::to_bytes(res.into_body()).await.unwrap(), &b"1234"[..]);
/// # })
/// ```
pub struct ResponseBuilder {
    head: Option<BoxedResponseHead>,
    err: Option<HttpError>,
}

impl ResponseBuilder {
    /// Create response builder
    ///
    /// # Examples
    /// ```
    /// use actix_http::{Response, ResponseBuilder, StatusCode};
    /// let res: Response<_> = ResponseBuilder::default().finish();
    /// assert_eq!(res.status(), StatusCode::OK);
    /// ```
    #[inline]
    pub fn new(status: StatusCode) -> Self {
        ResponseBuilder {
            head: Some(BoxedResponseHead::new(status)),
            err: None,
        }
    }

    /// Set HTTP status code of this response.
    ///
    /// # Examples
    /// ```
    /// use actix_http::{ResponseBuilder, StatusCode};
    /// let res = ResponseBuilder::default().status(StatusCode::NOT_FOUND).finish();
    /// assert_eq!(res.status(), StatusCode::NOT_FOUND);
    /// ```
    #[inline]
    pub fn status(&mut self, status: StatusCode) -> &mut Self {
        if let Some(parts) = self.inner() {
            parts.status = status;
        }
        self
    }

    /// Insert a header, replacing any that were set with an equivalent field name.
    ///
    /// # Examples
    /// ```
    /// use actix_http::{ResponseBuilder, header};
    ///
    /// let res = ResponseBuilder::default()
    ///     .insert_header((header::CONTENT_TYPE, mime::APPLICATION_JSON))
    ///     .insert_header(("X-TEST", "value"))
    ///     .finish();
    ///
    /// assert!(res.headers().contains_key("content-type"));
    /// assert!(res.headers().contains_key("x-test"));
    /// ```
    pub fn insert_header(&mut self, header: impl TryIntoHeaderPair) -> &mut Self {
        if let Some(parts) = self.inner() {
            match header.try_into_pair() {
                Ok((key, value)) => {
                    parts.headers.insert(key, value);
                }
                Err(err) => self.err = Some(err.into()),
            };
        }

        self
    }

    /// Append a header, keeping any that were set with an equivalent field name.
    ///
    /// # Examples
    /// ```
    /// use actix_http::{ResponseBuilder, header};
    ///
    /// let res = ResponseBuilder::default()
    ///     .append_header((header::CONTENT_TYPE, mime::APPLICATION_JSON))
    ///     .append_header(("X-TEST", "value1"))
    ///     .append_header(("X-TEST", "value2"))
    ///     .finish();
    ///
    /// assert_eq!(res.headers().get_all("content-type").count(), 1);
    /// assert_eq!(res.headers().get_all("x-test").count(), 2);
    /// ```
    pub fn append_header(&mut self, header: impl TryIntoHeaderPair) -> &mut Self {
        if let Some(parts) = self.inner() {
            match header.try_into_pair() {
                Ok((key, value)) => parts.headers.append(key, value),
                Err(err) => self.err = Some(err.into()),
            };
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

    /// Set connection type to `Upgrade`.
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

    /// Force-close connection, even if it is marked as keep-alive.
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
                Err(err) => self.err = Some(err.into()),
            };
        }
        self
    }

    /// Generate response with a wrapped body.
    ///
    /// This `ResponseBuilder` will be left in a useless state.
    pub fn body<B>(&mut self, body: B) -> Response<EitherBody<B>>
    where
        B: MessageBody + 'static,
    {
        match self.message_body(body) {
            Ok(res) => res.map_body(|_, body| EitherBody::left(body)),
            Err(err) => Response::from(err).map_body(|_, body| EitherBody::right(body)),
        }
    }

    /// Generate response with a body.
    ///
    /// This `ResponseBuilder` will be left in a useless state.
    pub fn message_body<B>(&mut self, body: B) -> Result<Response<B>, Error> {
        if let Some(err) = self.err.take() {
            return Err(Error::new_http().with_cause(err));
        }

        let head = self.head.take().expect("cannot reuse response builder");

        Ok(Response {
            head,
            body,
            extensions: RefCell::new(Extensions::new()),
        })
    }

    /// Generate response with an empty body.
    ///
    /// This `ResponseBuilder` will be left in a useless state.
    #[inline]
    pub fn finish(&mut self) -> Response<EitherBody<()>> {
        self.body(())
    }

    /// Create an owned `ResponseBuilder`, leaving the original in a useless state.
    pub fn take(&mut self) -> ResponseBuilder {
        ResponseBuilder {
            head: self.head.take(),
            err: self.err.take(),
        }
    }

    /// Get access to the inner response head if there has been no error.
    fn inner(&mut self) -> Option<&mut ResponseHead> {
        if self.err.is_some() {
            return None;
        }

        self.head.as_deref_mut()
    }
}

impl Default for ResponseBuilder {
    fn default() -> Self {
        Self::new(StatusCode::OK)
    }
}

/// Convert `Response` to a `ResponseBuilder`. Body get dropped.
impl<B> From<Response<B>> for ResponseBuilder {
    fn from(res: Response<B>) -> ResponseBuilder {
        ResponseBuilder {
            head: Some(res.head),
            err: None,
        }
    }
}

/// Convert `ResponseHead` to a `ResponseBuilder`
impl<'a> From<&'a ResponseHead> for ResponseBuilder {
    fn from(head: &'a ResponseHead) -> ResponseBuilder {
        let mut msg = BoxedResponseHead::new(head.status);
        msg.version = head.version;
        msg.reason = head.reason;

        for (k, v) in head.headers.iter() {
            msg.headers.append(k.clone(), v.clone());
        }

        msg.no_chunking(!head.chunked());

        ResponseBuilder {
            head: Some(msg),
            err: None,
        }
    }
}

impl fmt::Debug for ResponseBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let head = self.head.as_ref().unwrap();

        let res = writeln!(
            f,
            "\nResponseBuilder {:?} {}{}",
            head.version,
            head.status,
            head.reason.unwrap_or(""),
        );
        let _ = writeln!(f, "  headers:");
        for (key, val) in head.headers.iter() {
            let _ = writeln!(f, "    {:?}: {:?}", key, val);
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::header::{HeaderName, HeaderValue, CONTENT_TYPE};

    #[test]
    fn test_basic_builder() {
        let resp = Response::build(StatusCode::OK)
            .insert_header(("X-TEST", "value"))
            .finish();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_upgrade() {
        let resp = Response::build(StatusCode::OK)
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
        let resp = Response::build(StatusCode::OK).force_close().finish();
        assert!(!resp.keep_alive());
    }

    #[test]
    fn test_content_type() {
        let resp = Response::build(StatusCode::OK)
            .content_type("text/plain")
            .body(Bytes::new());
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "text/plain");

        let resp = Response::build(StatusCode::OK)
            .content_type(mime::APPLICATION_JAVASCRIPT_UTF_8)
            .body(Bytes::new());
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            "application/javascript; charset=utf-8"
        );
    }

    #[test]
    fn test_into_builder() {
        let mut resp: Response<_> = "test".into();
        assert_eq!(resp.status(), StatusCode::OK);

        resp.headers_mut().insert(
            HeaderName::from_static("cookie"),
            HeaderValue::from_static("cookie1=val100"),
        );

        let mut builder: ResponseBuilder = resp.into();
        let resp = builder.status(StatusCode::BAD_REQUEST).finish();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let cookie = resp.headers().get_all("Cookie").next().unwrap();
        assert_eq!(cookie.to_str().unwrap(), "cookie1=val100");
    }

    #[test]
    fn response_builder_header_insert_kv() {
        let mut res = Response::build(StatusCode::OK);
        res.insert_header(("Content-Type", "application/octet-stream"));
        let res = res.finish();

        assert_eq!(
            res.headers().get("Content-Type"),
            Some(&HeaderValue::from_static("application/octet-stream"))
        );
    }

    #[test]
    fn response_builder_header_insert_typed() {
        let mut res = Response::build(StatusCode::OK);
        res.insert_header((header::CONTENT_TYPE, mime::APPLICATION_OCTET_STREAM));
        let res = res.finish();

        assert_eq!(
            res.headers().get("Content-Type"),
            Some(&HeaderValue::from_static("application/octet-stream"))
        );
    }

    #[test]
    fn response_builder_header_append_kv() {
        let mut res = Response::build(StatusCode::OK);
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
        let mut res = Response::build(StatusCode::OK);
        res.append_header((header::CONTENT_TYPE, mime::APPLICATION_OCTET_STREAM));
        res.append_header((header::CONTENT_TYPE, mime::APPLICATION_JSON));
        let res = res.finish();

        let headers: Vec<_> = res.headers().get_all("Content-Type").cloned().collect();
        assert_eq!(headers.len(), 2);
        assert!(headers.contains(&HeaderValue::from_static("application/octet-stream")));
        assert!(headers.contains(&HeaderValue::from_static("application/json")));
    }
}
