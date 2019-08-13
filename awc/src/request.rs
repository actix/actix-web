use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::rc::Rc;
use std::time::Duration;
use std::{fmt, net};

use bytes::{BufMut, Bytes, BytesMut};
use futures::future::{err, Either};
use futures::{Future, Stream};
use percent_encoding::percent_encode;
use serde::Serialize;
use serde_json;
use tokio_timer::Timeout;

use actix_http::body::{Body, BodyStream};
use actix_http::cookie::{Cookie, CookieJar, USERINFO};
use actix_http::encoding::Decoder;
use actix_http::http::header::{self, ContentEncoding, Header, IntoHeaderValue};
use actix_http::http::{
    uri, ConnectionType, Error as HttpError, HeaderMap, HeaderName, HeaderValue,
    HttpTryFrom, Method, Uri, Version,
};
use actix_http::{Error, Payload, RequestHead};

use crate::error::{InvalidUrl, PayloadError, SendRequestError};
use crate::response::ClientResponse;
use crate::ClientConfig;

#[cfg(any(feature = "brotli", feature = "flate2-zlib", feature = "flate2-rust"))]
const HTTPS_ENCODING: &str = "br, gzip, deflate";
#[cfg(all(
    any(feature = "flate2-zlib", feature = "flate2-rust"),
    not(feature = "brotli")
))]
const HTTPS_ENCODING: &str = "gzip, deflate";

/// An HTTP Client request builder
///
/// This type can be used to construct an instance of `ClientRequest` through a
/// builder-like pattern.
///
/// ```rust
/// use futures::future::{Future, lazy};
/// use actix_rt::System;
///
/// fn main() {
///     System::new("test").block_on(lazy(|| {
///        awc::Client::new()
///           .get("http://www.rust-lang.org") // <- Create request builder
///           .header("User-Agent", "Actix-web")
///           .send()                          // <- Send http request
///           .map_err(|_| ())
///           .and_then(|response| {           // <- server http response
///                println!("Response: {:?}", response);
///                Ok(())
///           })
///     }));
/// }
/// ```
pub struct ClientRequest {
    pub(crate) head: RequestHead,
    err: Option<HttpError>,
    addr: Option<net::SocketAddr>,
    cookies: Option<CookieJar>,
    response_decompress: bool,
    timeout: Option<Duration>,
    config: Rc<ClientConfig>,
}

impl ClientRequest {
    /// Create new client request builder.
    pub(crate) fn new<U>(method: Method, uri: U, config: Rc<ClientConfig>) -> Self
    where
        Uri: HttpTryFrom<U>,
    {
        ClientRequest {
            config,
            head: RequestHead::default(),
            err: None,
            addr: None,
            cookies: None,
            timeout: None,
            response_decompress: true,
        }
        .method(method)
        .uri(uri)
    }

    /// Set HTTP URI of request.
    #[inline]
    pub fn uri<U>(mut self, uri: U) -> Self
    where
        Uri: HttpTryFrom<U>,
    {
        match Uri::try_from(uri) {
            Ok(uri) => self.head.uri = uri,
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Set socket address of the server.
    ///
    /// This address is used for connection. If address is not
    /// provided url's host name get resolved.
    pub fn address(mut self, addr: net::SocketAddr) -> Self {
        self.addr = Some(addr);
        self
    }

    /// Set HTTP method of this request.
    #[inline]
    pub fn method(mut self, method: Method) -> Self {
        self.head.method = method;
        self
    }

    #[doc(hidden)]
    /// Set HTTP version of this request.
    ///
    /// By default requests's HTTP version depends on network stream
    #[inline]
    pub fn version(mut self, version: Version) -> Self {
        self.head.version = version;
        self
    }

    #[inline]
    /// Returns request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    #[inline]
    /// Returns request's mutable headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.head.headers
    }

    /// Set a header.
    ///
    /// ```rust
    /// fn main() {
    /// # actix_rt::System::new("test").block_on(futures::future::lazy(|| {
    ///     let req = awc::Client::new()
    ///         .get("http://www.rust-lang.org")
    ///         .set(awc::http::header::Date::now())
    ///         .set(awc::http::header::ContentType(mime::TEXT_HTML));
    /// #   Ok::<_, ()>(())
    /// # }));
    /// }
    /// ```
    pub fn set<H: Header>(mut self, hdr: H) -> Self {
        match hdr.try_into() {
            Ok(value) => {
                self.head.headers.insert(H::name(), value);
            }
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Append a header.
    ///
    /// Header gets appended to existing header.
    /// To override header use `set_header()` method.
    ///
    /// ```rust
    /// use awc::{http, Client};
    ///
    /// fn main() {
    /// # actix_rt::System::new("test").block_on(futures::future::lazy(|| {
    ///     let req = Client::new()
    ///         .get("http://www.rust-lang.org")
    ///         .header("X-TEST", "value")
    ///         .header(http::header::CONTENT_TYPE, "application/json");
    /// #   Ok::<_, ()>(())
    /// # }));
    /// }
    /// ```
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match value.try_into() {
                Ok(value) => self.head.headers.append(key, value),
                Err(e) => self.err = Some(e.into()),
            },
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Insert a header, replaces existing header.
    pub fn set_header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match value.try_into() {
                Ok(value) => self.head.headers.insert(key, value),
                Err(e) => self.err = Some(e.into()),
            },
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Insert a header only if it is not yet set.
    pub fn set_header_if_none<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        match HeaderName::try_from(key) {
            Ok(key) => {
                if !self.head.headers.contains_key(&key) {
                    match value.try_into() {
                        Ok(value) => self.head.headers.insert(key, value),
                        Err(e) => self.err = Some(e.into()),
                    }
                }
            }
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Send headers in `Camel-Case` form.
    #[inline]
    pub fn camel_case(mut self) -> Self {
        self.head.set_camel_case_headers(true);
        self
    }

    /// Force close connection instead of returning it back to connections pool.
    /// This setting affect only http/1 connections.
    #[inline]
    pub fn force_close(mut self) -> Self {
        self.head.set_connection_type(ConnectionType::Close);
        self
    }

    /// Set request's content type
    #[inline]
    pub fn content_type<V>(mut self, value: V) -> Self
    where
        HeaderValue: HttpTryFrom<V>,
    {
        match HeaderValue::try_from(value) {
            Ok(value) => self.head.headers.insert(header::CONTENT_TYPE, value),
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Set content length
    #[inline]
    pub fn content_length(self, len: u64) -> Self {
        let mut wrt = BytesMut::new().writer();
        let _ = write!(wrt, "{}", len);
        self.header(header::CONTENT_LENGTH, wrt.get_mut().take().freeze())
    }

    /// Set HTTP basic authorization header
    pub fn basic_auth<U>(self, username: U, password: Option<&str>) -> Self
    where
        U: fmt::Display,
    {
        let auth = match password {
            Some(password) => format!("{}:{}", username, password),
            None => format!("{}:", username),
        };
        self.header(
            header::AUTHORIZATION,
            format!("Basic {}", base64::encode(&auth)),
        )
    }

    /// Set HTTP bearer authentication header
    pub fn bearer_auth<T>(self, token: T) -> Self
    where
        T: fmt::Display,
    {
        self.header(header::AUTHORIZATION, format!("Bearer {}", token))
    }

    /// Set a cookie
    ///
    /// ```rust
    /// # use actix_rt::System;
    /// # use futures::future::{lazy, Future};
    /// fn main() {
    ///     System::new("test").block_on(lazy(|| {
    ///         awc::Client::new().get("https://www.rust-lang.org")
    ///             .cookie(
    ///                 awc::http::Cookie::build("name", "value")
    ///                     .domain("www.rust-lang.org")
    ///                     .path("/")
    ///                     .secure(true)
    ///                     .http_only(true)
    ///                     .finish(),
    ///             )
    ///             .send()
    ///             .map_err(|_| ())
    ///             .and_then(|response| {
    ///                println!("Response: {:?}", response);
    ///                Ok(())
    ///             })
    ///     }));
    /// }
    /// ```
    pub fn cookie(mut self, cookie: Cookie<'_>) -> Self {
        if self.cookies.is_none() {
            let mut jar = CookieJar::new();
            jar.add(cookie.into_owned());
            self.cookies = Some(jar)
        } else {
            self.cookies.as_mut().unwrap().add(cookie.into_owned());
        }
        self
    }

    /// Disable automatic decompress of response's body
    pub fn no_decompress(mut self) -> Self {
        self.response_decompress = false;
        self
    }

    /// Set request timeout. Overrides client wide timeout setting.
    ///
    /// Request timeout is the total time before a response must be received.
    /// Default value is 5 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// This method calls provided closure with builder reference if
    /// value is `true`.
    pub fn if_true<F>(self, value: bool, f: F) -> Self
    where
        F: FnOnce(ClientRequest) -> ClientRequest,
    {
        if value {
            f(self)
        } else {
            self
        }
    }

    /// This method calls provided closure with builder reference if
    /// value is `Some`.
    pub fn if_some<T, F>(self, value: Option<T>, f: F) -> Self
    where
        F: FnOnce(T, ClientRequest) -> ClientRequest,
    {
        if let Some(val) = value {
            f(val, self)
        } else {
            self
        }
    }

    /// Complete request construction and send body.
    pub fn send_body<B>(
        mut self,
        body: B,
    ) -> impl Future<
        Item = ClientResponse<impl Stream<Item = Bytes, Error = PayloadError>>,
        Error = SendRequestError,
    >
    where
        B: Into<Body>,
    {
        if let Some(e) = self.err.take() {
            return Either::A(err(e.into()));
        }

        // validate uri
        let uri = &self.head.uri;
        if uri.host().is_none() {
            return Either::A(err(InvalidUrl::MissingHost.into()));
        } else if uri.scheme_part().is_none() {
            return Either::A(err(InvalidUrl::MissingScheme.into()));
        } else if let Some(scheme) = uri.scheme_part() {
            match scheme.as_str() {
                "http" | "ws" | "https" | "wss" => (),
                _ => return Either::A(err(InvalidUrl::UnknownScheme.into())),
            }
        } else {
            return Either::A(err(InvalidUrl::UnknownScheme.into()));
        }

        // set cookies
        if let Some(ref mut jar) = self.cookies {
            let mut cookie = String::new();
            for c in jar.delta() {
                let name = percent_encode(c.name().as_bytes(), USERINFO);
                let value = percent_encode(c.value().as_bytes(), USERINFO);
                let _ = write!(&mut cookie, "; {}={}", name, value);
            }
            self.head.headers.insert(
                header::COOKIE,
                HeaderValue::from_str(&cookie.as_str()[2..]).unwrap(),
            );
        }

        let mut slf = self;

        // enable br only for https
        #[cfg(any(
            feature = "brotli",
            feature = "flate2-zlib",
            feature = "flate2-rust"
        ))]
        {
            if slf.response_decompress {
                let https = slf
                    .head
                    .uri
                    .scheme_part()
                    .map(|s| s == &uri::Scheme::HTTPS)
                    .unwrap_or(true);

                if https {
                    slf = slf.set_header_if_none(header::ACCEPT_ENCODING, HTTPS_ENCODING)
                } else {
                    #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
                    {
                        slf = slf
                            .set_header_if_none(header::ACCEPT_ENCODING, "gzip, deflate")
                    }
                };
            }
        }

        let head = slf.head;
        let config = slf.config.as_ref();
        let response_decompress = slf.response_decompress;

        let fut = config
            .connector
            .borrow_mut()
            .send_request(head, body.into(), slf.addr)
            .map(move |res| {
                res.map_body(|head, payload| {
                    if response_decompress {
                        Payload::Stream(Decoder::from_headers(payload, &head.headers))
                    } else {
                        Payload::Stream(Decoder::new(payload, ContentEncoding::Identity))
                    }
                })
            });

        // set request timeout
        if let Some(timeout) = slf.timeout.or_else(|| config.timeout) {
            Either::B(Either::A(Timeout::new(fut, timeout).map_err(|e| {
                if let Some(e) = e.into_inner() {
                    e
                } else {
                    SendRequestError::Timeout
                }
            })))
        } else {
            Either::B(Either::B(fut))
        }
    }

    /// Set a JSON body and generate `ClientRequest`
    pub fn send_json<T: Serialize>(
        self,
        value: &T,
    ) -> impl Future<
        Item = ClientResponse<impl Stream<Item = Bytes, Error = PayloadError>>,
        Error = SendRequestError,
    > {
        let body = match serde_json::to_string(value) {
            Ok(body) => body,
            Err(e) => return Either::A(err(Error::from(e).into())),
        };

        // set content-type
        let slf = self.set_header_if_none(header::CONTENT_TYPE, "application/json");

        Either::B(slf.send_body(Body::Bytes(Bytes::from(body))))
    }

    /// Set a urlencoded body and generate `ClientRequest`
    ///
    /// `ClientRequestBuilder` can not be used after this call.
    pub fn send_form<T: Serialize>(
        self,
        value: &T,
    ) -> impl Future<
        Item = ClientResponse<impl Stream<Item = Bytes, Error = PayloadError>>,
        Error = SendRequestError,
    > {
        let body = match serde_urlencoded::to_string(value) {
            Ok(body) => body,
            Err(e) => return Either::A(err(Error::from(e).into())),
        };

        // set content-type
        let slf = self.set_header_if_none(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        );

        Either::B(slf.send_body(Body::Bytes(Bytes::from(body))))
    }

    /// Set an streaming body and generate `ClientRequest`.
    pub fn send_stream<S, E>(
        self,
        stream: S,
    ) -> impl Future<
        Item = ClientResponse<impl Stream<Item = Bytes, Error = PayloadError>>,
        Error = SendRequestError,
    >
    where
        S: Stream<Item = Bytes, Error = E> + 'static,
        E: Into<Error> + 'static,
    {
        self.send_body(Body::from_message(BodyStream::new(stream)))
    }

    /// Set an empty body and generate `ClientRequest`.
    pub fn send(
        self,
    ) -> impl Future<
        Item = ClientResponse<impl Stream<Item = Bytes, Error = PayloadError>>,
        Error = SendRequestError,
    > {
        self.send_body(Body::Empty)
    }
}

impl fmt::Debug for ClientRequest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "\nClientRequest {:?} {}:{}",
            self.head.version, self.head.method, self.head.uri
        )?;
        writeln!(f, "  headers:")?;
        for (key, val) in self.head.headers.iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;
    use crate::Client;

    #[test]
    fn test_debug() {
        let request = Client::new().get("/").header("x-test", "111");
        let repr = format!("{:?}", request);
        assert!(repr.contains("ClientRequest"));
        assert!(repr.contains("x-test"));
    }

    #[test]
    fn test_basics() {
        let mut req = Client::new()
            .put("/")
            .version(Version::HTTP_2)
            .set(header::Date(SystemTime::now().into()))
            .content_type("plain/text")
            .if_true(true, |req| req.header(header::SERVER, "awc"))
            .if_true(false, |req| req.header(header::EXPECT, "awc"))
            .if_some(Some("server"), |val, req| {
                req.header(header::USER_AGENT, val)
            })
            .if_some(Option::<&str>::None, |_, req| {
                req.header(header::ALLOW, "1")
            })
            .content_length(100);
        assert!(req.headers().contains_key(header::CONTENT_TYPE));
        assert!(req.headers().contains_key(header::DATE));
        assert!(req.headers().contains_key(header::SERVER));
        assert!(req.headers().contains_key(header::USER_AGENT));
        assert!(!req.headers().contains_key(header::ALLOW));
        assert!(!req.headers().contains_key(header::EXPECT));
        assert_eq!(req.head.version, Version::HTTP_2);
        let _ = req.headers_mut();
        let _ = req.send_body("");
    }

    #[test]
    fn test_client_header() {
        let req = Client::build()
            .header(header::CONTENT_TYPE, "111")
            .finish()
            .get("/");

        assert_eq!(
            req.head
                .headers
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            "111"
        );
    }

    #[test]
    fn test_client_header_override() {
        let req = Client::build()
            .header(header::CONTENT_TYPE, "111")
            .finish()
            .get("/")
            .set_header(header::CONTENT_TYPE, "222");

        assert_eq!(
            req.head
                .headers
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            "222"
        );
    }

    #[test]
    fn client_basic_auth() {
        let req = Client::new()
            .get("/")
            .basic_auth("username", Some("password"));
        assert_eq!(
            req.head
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Basic dXNlcm5hbWU6cGFzc3dvcmQ="
        );

        let req = Client::new().get("/").basic_auth("username", None);
        assert_eq!(
            req.head
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Basic dXNlcm5hbWU6"
        );
    }

    #[test]
    fn client_bearer_auth() {
        let req = Client::new().get("/").bearer_auth("someS3cr3tAutht0k3n");
        assert_eq!(
            req.head
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer someS3cr3tAutht0k3n"
        );
    }
}
