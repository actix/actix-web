use std::{
    cell::{Ref, RefMut},
    convert::TryInto,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::{
    body::{Body, BodyStream},
    http::{
        header::{self, HeaderName, IntoHeaderPair, IntoHeaderValue},
        ConnectionType, Error as HttpError, StatusCode,
    },
    Extensions, Response, ResponseHead,
};
use bytes::Bytes;
use futures_core::Stream;
use serde::Serialize;

#[cfg(feature = "cookies")]
use actix_http::http::header::HeaderValue;
#[cfg(feature = "cookies")]
use cookie::{Cookie, CookieJar};

use crate::{
    error::{Error, JsonPayloadError},
    HttpResponse,
};

/// An HTTP response builder.
///
/// This type can be used to construct an instance of `Response` through a builder-like pattern.
pub struct HttpResponseBuilder {
    head: Option<ResponseHead>,
    err: Option<HttpError>,
    #[cfg(feature = "cookies")]
    cookies: Option<CookieJar>,
}

impl HttpResponseBuilder {
    #[inline]
    /// Create response builder
    pub fn new(status: StatusCode) -> Self {
        Self {
            head: Some(ResponseHead::new(status)),
            err: None,
            #[cfg(feature = "cookies")]
            cookies: None,
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
    pub fn insert_header<H>(&mut self, header: H) -> &mut Self
    where
        H: IntoHeaderPair,
    {
        if let Some(parts) = self.inner() {
            match header.try_into_header_pair() {
                Ok((key, value)) => {
                    parts.headers.insert(key, value);
                }
                Err(e) => self.err = Some(e.into()),
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
    pub fn append_header<H>(&mut self, header: H) -> &mut Self
    where
        H: IntoHeaderPair,
    {
        if let Some(parts) = self.inner() {
            match header.try_into_header_pair() {
                Ok((key, value)) => parts.headers.append(key, value),
                Err(e) => self.err = Some(e.into()),
            };
        }

        self
    }

    /// Replaced with [`Self::insert_header()`].
    #[deprecated(
        since = "4.0.0",
        note = "Replaced with `insert_header((key, value))`. Will be removed in v5."
    )]
    pub fn set_header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: TryInto<HeaderName>,
        K::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        if self.err.is_some() {
            return self;
        }

        match (key.try_into(), value.try_into_value()) {
            (Ok(name), Ok(value)) => return self.insert_header((name, value)),
            (Err(err), _) => self.err = Some(err.into()),
            (_, Err(err)) => self.err = Some(err.into()),
        }

        self
    }

    /// Replaced with [`Self::append_header()`].
    #[deprecated(
        since = "4.0.0",
        note = "Replaced with `append_header((key, value))`. Will be removed in v5."
    )]
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: TryInto<HeaderName>,
        K::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        if self.err.is_some() {
            return self;
        }

        match (key.try_into(), value.try_into_value()) {
            (Ok(name), Ok(value)) => return self.append_header((name, value)),
            (Err(err), _) => self.err = Some(err.into()),
            (_, Err(err)) => self.err = Some(err.into()),
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
        V: IntoHeaderValue,
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
        V: IntoHeaderValue,
    {
        if let Some(parts) = self.inner() {
            match value.try_into_value() {
                Ok(value) => {
                    parts.headers.insert(header::CONTENT_TYPE, value);
                }
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set a cookie.
    ///
    /// ```
    /// use actix_web::{HttpResponse, cookie::Cookie};
    ///
    /// HttpResponse::Ok()
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
    #[cfg(feature = "cookies")]
    pub fn cookie<'c>(&mut self, cookie: Cookie<'c>) -> &mut Self {
        if self.cookies.is_none() {
            let mut jar = CookieJar::new();
            jar.add(cookie.into_owned());
            self.cookies = Some(jar)
        } else {
            self.cookies.as_mut().unwrap().add(cookie.into_owned());
        }
        self
    }

    /// Remove cookie.
    ///
    /// A `Set-Cookie` header is added that will delete a cookie with the same name from the client.
    ///
    /// ```
    /// use actix_web::{HttpRequest, HttpResponse, Responder};
    ///
    /// async fn handler(req: HttpRequest) -> impl Responder {
    ///     let mut builder = HttpResponse::Ok();
    ///
    ///     if let Some(ref cookie) = req.cookie("name") {
    ///         builder.del_cookie(cookie);
    ///     }
    ///
    ///     builder.finish()
    /// }
    /// ```
    #[cfg(feature = "cookies")]
    pub fn del_cookie(&mut self, cookie: &Cookie<'_>) -> &mut Self {
        if self.cookies.is_none() {
            self.cookies = Some(CookieJar::new())
        }
        let jar = self.cookies.as_mut().unwrap();
        let cookie = cookie.clone().into_owned();
        jar.add_original(cookie.clone());
        jar.remove(cookie);
        self
    }

    /// Responses extensions
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        let head = self.head.as_ref().expect("cannot reuse response builder");
        head.extensions()
    }

    /// Mutable reference to a the response's extensions
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<'_, Extensions> {
        let head = self.head.as_ref().expect("cannot reuse response builder");
        head.extensions_mut()
    }

    /// Set a body and generate `Response`.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    #[inline]
    pub fn body<B: Into<Body>>(&mut self, body: B) -> HttpResponse {
        self.message_body(body.into())
    }

    /// Set a body and generate `Response`.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    pub fn message_body<B>(&mut self, body: B) -> HttpResponse<B> {
        if let Some(err) = self.err.take() {
            return HttpResponse::from_error(Error::from(err)).into_body();
        }

        // allow unused mut when cookies feature is disabled
        #[allow(unused_mut)]
        let mut head = self.head.take().expect("cannot reuse response builder");

        let mut res = HttpResponse::with_body(StatusCode::OK, body);
        *res.head_mut() = head;

        #[cfg(feature = "cookies")]
        if let Some(ref jar) = self.cookies {
            for cookie in jar.delta() {
                match HeaderValue::from_str(&cookie.to_string()) {
                    Ok(val) => res.headers_mut().append(header::SET_COOKIE, val),
                    Err(err) => return HttpResponse::from_error(Error::from(err)).into_body(),
                };
            }
        }

        res
    }

    /// Set a streaming body and generate `Response`.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    #[inline]
    pub fn streaming<S, E>(&mut self, stream: S) -> HttpResponse
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + 'static,
        E: Into<Error> + 'static,
    {
        self.body(Body::from_message(BodyStream::new(stream)))
    }

    /// Set a json body and generate `Response`
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

                self.body(Body::from(body))
            }
            Err(err) => HttpResponse::from_error(JsonPayloadError::Serialize(err).into()),
        }
    }

    /// Set an empty body and generate `Response`
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    #[inline]
    pub fn finish(&mut self) -> HttpResponse {
        self.body(Body::Empty)
    }

    /// This method construct new `HttpResponseBuilder`
    pub fn take(&mut self) -> Self {
        Self {
            head: self.head.take(),
            err: self.err.take(),
            #[cfg(feature = "cookies")]
            cookies: self.cookies.take(),
        }
    }

    #[inline]
    fn inner(&mut self) -> Option<&mut ResponseHead> {
        if self.err.is_some() {
            return None;
        }

        self.head.as_mut()
    }
}

impl From<HttpResponseBuilder> for HttpResponse {
    fn from(mut builder: HttpResponseBuilder) -> Self {
        builder.finish()
    }
}

impl From<HttpResponseBuilder> for Response<Body> {
    fn from(mut builder: HttpResponseBuilder) -> Self {
        builder.finish().into()
    }
}

impl Future for HttpResponseBuilder {
    type Output = Result<HttpResponse, Error>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        eprintln!("httpresponse future error");
        Poll::Ready(Ok(self.finish()))
    }
}

#[cfg(test)]
mod tests {
    use actix_http::body;

    use super::*;
    use crate::{
        dev::Body,
        http::{
            header::{self, HeaderValue, CONTENT_TYPE},
            StatusCode,
        },
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
            .body(Body::Empty);
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "text/plain")
    }

    #[actix_rt::test]
    async fn test_json() {
        let mut resp = HttpResponse::Ok().json(vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("application/json"));
        assert_eq!(
            body::to_bytes(resp.take_body()).await.unwrap().as_ref(),
            br#"["v1","v2","v3"]"#
        );

        let mut resp = HttpResponse::Ok().json(&["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("application/json"));
        assert_eq!(
            body::to_bytes(resp.take_body()).await.unwrap().as_ref(),
            br#"["v1","v2","v3"]"#
        );

        // content type override
        let mut resp = HttpResponse::Ok()
            .insert_header((CONTENT_TYPE, "text/json"))
            .json(&vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("text/json"));
        assert_eq!(
            body::to_bytes(resp.take_body()).await.unwrap().as_ref(),
            br#"["v1","v2","v3"]"#
        );
    }

    #[actix_rt::test]
    async fn test_serde_json_in_body() {
        let mut resp = HttpResponse::Ok().body(
            serde_json::to_vec(&serde_json::json!({ "test-key": "test-value" })).unwrap(),
        );

        assert_eq!(
            body::to_bytes(resp.take_body()).await.unwrap().as_ref(),
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
