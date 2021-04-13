use std::{
    cell::{Ref, RefMut},
    fmt,
    future::Future,
    mem,
    pin::Pin,
    str,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::Stream;

use crate::{
    body::{Body, BodyStream, ResponseBody},
    error::Error,
    extensions::Extensions,
    header::{IntoHeaderPair, IntoHeaderValue},
    http::{header, Error as HttpError, StatusCode},
    message::{BoxedResponseHead, ConnectionType, ResponseHead},
    response::Response,
};

/// An HTTP response builder.
///
/// This type can be used to construct an instance of `Response` through a builder-like pattern.
pub struct ResponseBuilder {
    inner: Result<BoxedResponseHead, HttpError>,
}

impl ResponseBuilder {
    #[inline]
    /// Create response builder
    pub fn new(status: StatusCode) -> Self {
        ResponseBuilder {
            inner: Ok(BoxedResponseHead::new(status)),
        }
    }

    /// Set HTTP status code of this response.
    #[inline]
    pub fn status(mut self, status: StatusCode) -> Self {
        if let Some(parts) = self.inner() {
            parts.status = status;
        }
        self
    }

    /// Insert a header, replacing any that were set with an equivalent field name.
    ///
    /// ```
    /// # use actix_http::Response;
    /// use actix_http::http::header;
    ///
    /// Response::Ok()
    ///     .insert_header((header::CONTENT_TYPE, mime::APPLICATION_JSON))
    ///     .insert_header(("X-TEST", "value"))
    ///     .finish();
    /// ```
    pub fn insert_header<H>(mut self, header: H) -> Self
    where
        H: IntoHeaderPair,
    {
        if let Some(parts) = self.inner() {
            match header.try_into_header_pair() {
                Ok((key, value)) => {
                    parts.headers.insert(key, value);
                }
                Err(err) => self.inner = Err(err.into()),
            };
        }

        self
    }

    /// Append a header, keeping any that were set with an equivalent field name.
    ///
    /// ```
    /// # use actix_http::Response;
    /// use actix_http::http::header;
    ///
    /// Response::Ok()
    ///     .append_header((header::CONTENT_TYPE, mime::APPLICATION_JSON))
    ///     .append_header(("X-TEST", "value1"))
    ///     .append_header(("X-TEST", "value2"))
    ///     .finish();
    /// ```
    pub fn append_header<H>(mut self, header: H) -> Self
    where
        H: IntoHeaderPair,
    {
        if let Some(parts) = self.inner() {
            match header.try_into_header_pair() {
                Ok((key, value)) => parts.headers.append(key, value),
                Err(err) => self.inner = Err(err.into()),
            };
        }

        self
    }

    /// Set the custom reason for the response.
    #[inline]
    pub fn reason(mut self, reason: &'static str) -> Self {
        if let Some(parts) = self.inner() {
            parts.reason = Some(reason);
        }
        self
    }

    /// Set connection type to KeepAlive
    #[inline]
    pub fn keep_alive(self) -> Self {
        if let Some(parts) = self.inner() {
            parts.set_connection_type(ConnectionType::KeepAlive);
        }
        self
    }

    /// Set connection type to Upgrade
    #[inline]
    pub fn upgrade<V>(mut self, value: V) -> Self
    where
        V: IntoHeaderValue,
    {
        if let Some(parts) = self.inner() {
            parts.set_connection_type(ConnectionType::Upgrade);
        }

        if let Ok(value) = value.try_into_value() {
            self.insert_header((header::UPGRADE, value));
        }

        res
    }

    /// Force close connection, even if it is marked as keep-alive
    #[inline]
    pub fn force_close(mut self) -> Self {
        if let Some(parts) = self.inner() {
            parts.set_connection_type(ConnectionType::Close);
        }
        self
    }

    /// Disable chunked transfer encoding for HTTP/1.1 streaming responses.
    #[inline]
    pub fn no_chunking(self, len: u64) -> Self {
        let mut buf = itoa::Buffer::new();
        let mut res = self.insert_header((header::CONTENT_LENGTH, buf.format(len)));

        if let Some(head) = res.inner() {
            head.no_chunking(true);
        }

        res
    }

    /// Set response content type.
    #[inline]
    pub fn content_type<V>(mut self, value: V) -> Self
    where
        V: IntoHeaderValue,
    {
        if let Some(head) = self.inner() {
            match value.try_into_value() {
                Ok(value) => {
                    head.headers.insert(header::CONTENT_TYPE, value);
                }
                Err(err) => self.inner = Err(err.into()),
            };
        }
        self
    }

    /// Responses extensions
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        let head = self.inner.as_ref().expect("cannot reuse response builder");
        head.extensions.borrow()
    }

    /// Mutable reference to a the response's extensions
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<'_, Extensions> {
        let head = self.inner.as_ref().expect("cannot reuse response builder");
        head.extensions.borrow_mut()
    }

    /// Creates an owned response builder, leaving a default-ish builder in it's place.
    ///
    /// Useful under the assumption the original builder will be dropped immediately.
    ///
    /// If the builder contains an error, it will be passed to the new, owned builder.
    pub fn take(&mut self) -> ResponseBuilder {
        let res = BoxedResponseHead::new(StatusCode::INTERNAL_SERVER_ERROR);
        let inner = mem::replace(&mut self.inner, Ok(res));

        ResponseBuilder { inner }
    }

    /// Set a body and generate `Response`.
    ///
    /// `ResponseBuilder` can not be used after this call.
    fn with_body<B>(self, body: B) -> Result<Response<B>, Error> {
        match self.inner {
            Ok(head) => Ok(Response {
                head,
                body: ResponseBody::Body(body),
                error: None,
            }),
            Err(err) => Err(Error::from(err)),
        }
    }

    /// Consume builder and generate response with given body.
    #[inline]
    pub fn body<B>(self, body: B) -> Result<Response<B>, Error> {
        self.with_body(body.into())
    }

    /// Consume builder and generate response with given stream as body.
    #[inline]
    pub fn streaming<S, E>(self, stream: S) -> Result<Response<BodyStream<S>>, Error>
    where
        S: Stream<Item = Result<Bytes, E>> + 'static,
        E: Into<Error> + 'static,
    {
        self.body(BodyStream::new(stream))
    }

    /// Consume builder and generate response with empty body.
    #[inline]
    pub fn finish(self) -> Result<Response<Body>, Error> {
        self.body(Body::Empty)
    }

    /// Consume builder and generate response with empty body, converting errors into responses.
    #[inline]
    pub fn complete(self) -> Response<Body> {
        self.body(Body::Empty).into()
    }

    /// Access to contained response when there is no error.
    fn inner(&mut self) -> Option<&mut ResponseHead> {
        self.inner.as_mut().ok().map(|head| &mut **head)
    }
}

/// Convert `Response` to a `ResponseBuilder`. Body get dropped.
impl<B> From<Response<B>> for ResponseBuilder {
    fn from(res: Response<B>) -> ResponseBuilder {
        ResponseBuilder {
            inner: Ok(res.head),
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

        ResponseBuilder { inner: Ok(msg) }
    }
}

impl Future for ResponseBuilder {
    type Output = Result<Response<Body>, Error>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.take().finish())
    }
}

impl fmt::Debug for ResponseBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let head = self.inner.as_ref().unwrap();

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
    use super::*;
    use crate::body::Body;
    use crate::http::header::{HeaderName, HeaderValue, CONTENT_TYPE};

    #[test]
    fn test_basic_builder() {
        let resp = Response::builder(StatusCode::OK)
            .insert_header(("X-TEST", "value"))
            .complete();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_upgrade() {
        let resp = Response::builder(StatusCode::OK)
            .upgrade("websocket")
            .complete();
        assert!(resp.upgrade());
        assert_eq!(
            resp.headers().get(header::UPGRADE).unwrap(),
            HeaderValue::from_static("websocket")
        );
    }

    #[test]
    fn test_force_close() {
        let resp = Response::builder(StatusCode::OK)
            .force_close()
            .finish()
            .unwrap();
        assert!(!resp.keep_alive())
    }

    #[test]
    fn test_content_type() {
        let resp = Response::builder(StatusCode::OK)
            .content_type("text/plain")
            .body(Body::Empty)
            .unwrap();
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "text/plain")
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
        let resp = builder.status(StatusCode::BAD_REQUEST).finish().unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let cookie = resp.headers().get_all("Cookie").next().unwrap();
        assert_eq!(cookie.to_str().unwrap(), "cookie1=val100");
    }

    #[test]
    fn response_builder_header_insert_kv() {
        let res = Response::builder(StatusCode::OK)
            .insert_header(("Content-Type", "application/octet-stream"))
            .take()
            .finish()
            .unwrap();

        assert_eq!(
            res.headers().get("Content-Type"),
            Some(&HeaderValue::from_static("application/octet-stream"))
        );
    }

    #[test]
    fn response_builder_header_insert_typed() {
        let res = Response::builder(StatusCode::OK)
            .insert_header((header::CONTENT_TYPE, mime::APPLICATION_OCTET_STREAM))
            .take()
            .finish()
            .unwrap();

        assert_eq!(
            res.headers().get("Content-Type"),
            Some(&HeaderValue::from_static("application/octet-stream"))
        );
    }

    #[test]
    fn response_builder_header_append_kv() {
        let mut res = Response::builder(StatusCode::OK)
            .append_header(("Content-Type", "application/octet-stream"))
            .append_header(("Content-Type", "application/json"))
            .take()
            .finish()
            .unwrap();

        let headers: Vec<_> = res.headers().get_all("Content-Type").cloned().collect();
        assert_eq!(headers.len(), 2);
        assert!(headers.contains(&HeaderValue::from_static("application/octet-stream")));
        assert!(headers.contains(&HeaderValue::from_static("application/json")));
    }

    #[test]
    fn response_builder_header_append_typed() {
        let mut res = Response::builder(StatusCode::OK);
        res.append_header((header::CONTENT_TYPE, mime::APPLICATION_OCTET_STREAM));
        res.append_header((header::CONTENT_TYPE, mime::APPLICATION_JSON));
        let res = res.finish().unwrap();

        let headers: Vec<_> = res.headers().get_all("Content-Type").cloned().collect();
        assert_eq!(headers.len(), 2);
        assert!(headers.contains(&HeaderValue::from_static("application/octet-stream")));
        assert!(headers.contains(&HeaderValue::from_static("application/json")));
    }
}
