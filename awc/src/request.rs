use std::fmt;
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::rc::Rc;
use std::time::Duration;

use bytes::{BufMut, Bytes, BytesMut};
use futures::future::{err, Either};
use futures::{Future, Stream};
use percent_encoding::{percent_encode, USERINFO_ENCODE_SET};
use serde::Serialize;
use serde_json;
use tokio_timer::Timeout;

use actix_http::body::{Body, BodyStream};
use actix_http::cookie::{Cookie, CookieJar};
use actix_http::encoding::Decoder;
use actix_http::http::header::{self, ContentEncoding, Header, IntoHeaderValue};
use actix_http::http::{
    uri, ConnectionType, Error as HttpError, HeaderName, HeaderValue, HttpTryFrom,
    Method, Uri, Version,
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
    pub(crate) head: Option<RequestHead>,
    err: Option<HttpError>,
    cookies: Option<CookieJar>,
    default_headers: bool,
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
        let mut req = ClientRequest {
            config,
            head: Some(RequestHead::default()),
            err: None,
            cookies: None,
            timeout: None,
            default_headers: true,
            response_decompress: true,
        };
        req.method(method).uri(uri);
        req
    }

    /// Set HTTP URI of request.
    #[inline]
    pub fn uri<U>(&mut self, uri: U) -> &mut Self
    where
        Uri: HttpTryFrom<U>,
    {
        if let Some(head) = parts(&mut self.head, &self.err) {
            match Uri::try_from(uri) {
                Ok(uri) => head.uri = uri,
                Err(e) => self.err = Some(e.into()),
            }
        }
        self
    }

    /// Set HTTP method of this request.
    #[inline]
    pub fn method(&mut self, method: Method) -> &mut Self {
        if let Some(head) = parts(&mut self.head, &self.err) {
            head.method = method;
        }
        self
    }

    #[doc(hidden)]
    /// Set HTTP version of this request.
    ///
    /// By default requests's HTTP version depends on network stream
    #[inline]
    pub fn version(&mut self, version: Version) -> &mut Self {
        if let Some(head) = parts(&mut self.head, &self.err) {
            head.version = version;
        }
        self
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
    pub fn set<H: Header>(&mut self, hdr: H) -> &mut Self {
        if let Some(head) = parts(&mut self.head, &self.err) {
            match hdr.try_into() {
                Ok(value) => {
                    head.headers.insert(H::name(), value);
                }
                Err(e) => self.err = Some(e.into()),
            }
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
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Some(head) = parts(&mut self.head, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => match value.try_into() {
                    Ok(value) => {
                        head.headers.append(key, value);
                    }
                    Err(e) => self.err = Some(e.into()),
                },
                Err(e) => self.err = Some(e.into()),
            }
        }
        self
    }

    /// Insert a header, replaces existing header.
    pub fn set_header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Some(head) = parts(&mut self.head, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => match value.try_into() {
                    Ok(value) => {
                        head.headers.insert(key, value);
                    }
                    Err(e) => self.err = Some(e.into()),
                },
                Err(e) => self.err = Some(e.into()),
            }
        }
        self
    }

    /// Insert a header only if it is not yet set.
    pub fn set_header_if_none<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Some(head) = parts(&mut self.head, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => {
                    if !head.headers.contains_key(&key) {
                        match value.try_into() {
                            Ok(value) => {
                                head.headers.insert(key, value);
                            }
                            Err(e) => self.err = Some(e.into()),
                        }
                    }
                }
                Err(e) => self.err = Some(e.into()),
            }
        }
        self
    }

    /// Close connection instead of returning it back to connections pool.
    /// This setting affect only http/1 connections.
    #[inline]
    pub fn close_connection(&mut self) -> &mut Self {
        if let Some(head) = parts(&mut self.head, &self.err) {
            head.set_connection_type(ConnectionType::Close);
        }
        self
    }

    /// Set request's content type
    #[inline]
    pub fn content_type<V>(&mut self, value: V) -> &mut Self
    where
        HeaderValue: HttpTryFrom<V>,
    {
        if let Some(head) = parts(&mut self.head, &self.err) {
            match HeaderValue::try_from(value) {
                Ok(value) => {
                    let _ = head.headers.insert(header::CONTENT_TYPE, value);
                }
                Err(e) => self.err = Some(e.into()),
            }
        }
        self
    }

    /// Set content length
    #[inline]
    pub fn content_length(&mut self, len: u64) -> &mut Self {
        let mut wrt = BytesMut::new().writer();
        let _ = write!(wrt, "{}", len);
        self.header(header::CONTENT_LENGTH, wrt.get_mut().take().freeze())
    }

    /// Set HTTP basic authorization header
    pub fn basic_auth<U>(&mut self, username: U, password: Option<&str>) -> &mut Self
    where
        U: fmt::Display,
    {
        let auth = match password {
            Some(password) => format!("{}:{}", username, password),
            None => format!("{}", username),
        };
        self.header(
            header::AUTHORIZATION,
            format!("Basic {}", base64::encode(&auth)),
        )
    }

    /// Set HTTP bearer authentication header
    pub fn bearer_auth<T>(&mut self, token: T) -> &mut Self
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

    /// Do not add default request headers.
    /// By default `Date` and `User-Agent` headers are set.
    pub fn no_default_headers(&mut self) -> &mut Self {
        self.default_headers = false;
        self
    }

    /// Disable automatic decompress of response's body
    pub fn no_decompress(&mut self) -> &mut Self {
        self.response_decompress = false;
        self
    }

    /// Set request timeout. Overrides client wide timeout setting.
    ///
    /// Request timeout is the total time before a response must be received.
    /// Default value is 5 seconds.
    pub fn timeout(&mut self, timeout: Duration) -> &mut Self {
        self.timeout = Some(timeout);
        self
    }

    /// This method calls provided closure with builder reference if
    /// value is `true`.
    pub fn if_true<F>(&mut self, value: bool, f: F) -> &mut Self
    where
        F: FnOnce(&mut ClientRequest),
    {
        if value {
            f(self);
        }
        self
    }

    /// This method calls provided closure with builder reference if
    /// value is `Some`.
    pub fn if_some<T, F>(&mut self, value: Option<T>, f: F) -> &mut Self
    where
        F: FnOnce(T, &mut ClientRequest),
    {
        if let Some(val) = value {
            f(val, self);
        }
        self
    }

    /// Complete request construction and send body.
    pub fn send_body<B>(
        &mut self,
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

        let mut head = self.head.take().expect("cannot reuse response builder");

        // validate uri
        let uri = &head.uri;
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

        // set default headers
        if self.default_headers {
            // set request host header
            if let Some(host) = head.uri.host() {
                if !head.headers.contains_key(header::HOST) {
                    let mut wrt = BytesMut::with_capacity(host.len() + 5).writer();

                    let _ = match head.uri.port_u16() {
                        None | Some(80) | Some(443) => write!(wrt, "{}", host),
                        Some(port) => write!(wrt, "{}:{}", host, port),
                    };

                    match wrt.get_mut().take().freeze().try_into() {
                        Ok(value) => {
                            head.headers.insert(header::HOST, value);
                        }
                        Err(e) => return Either::A(err(HttpError::from(e).into())),
                    }
                }
            }

            // user agent
            self.set_header_if_none(
                header::USER_AGENT,
                concat!("awc/", env!("CARGO_PKG_VERSION")),
            );
        }

        // enable br only for https
        let https = head
            .uri
            .scheme_part()
            .map(|s| s == &uri::Scheme::HTTPS)
            .unwrap_or(true);

        #[cfg(any(
            feature = "brotli",
            feature = "flate2-zlib",
            feature = "flate2-rust"
        ))]
        {
            if https {
                self.set_header_if_none(header::ACCEPT_ENCODING, HTTPS_ENCODING);
            } else {
                #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
                {
                    self.set_header_if_none(header::ACCEPT_ENCODING, "gzip, deflate");
                }
            }
        }

        // set cookies
        if let Some(ref mut jar) = self.cookies {
            let mut cookie = String::new();
            for c in jar.delta() {
                let name = percent_encode(c.name().as_bytes(), USERINFO_ENCODE_SET);
                let value = percent_encode(c.value().as_bytes(), USERINFO_ENCODE_SET);
                let _ = write!(&mut cookie, "; {}={}", name, value);
            }
            head.headers.insert(
                header::COOKIE,
                HeaderValue::from_str(&cookie.as_str()[2..]).unwrap(),
            );
        }

        let config = self.config.as_ref();
        let response_decompress = self.response_decompress;

        let fut = config
            .connector
            .borrow_mut()
            .send_request(head, body.into())
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
        if let Some(timeout) = self.timeout.or_else(|| config.timeout.clone()) {
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
        &mut self,
        value: &T,
    ) -> impl Future<
        Item = ClientResponse<impl Stream<Item = Bytes, Error = PayloadError>>,
        Error = SendRequestError,
    > {
        let body = match serde_json::to_string(&value) {
            Ok(body) => body,
            Err(e) => return Either::A(err(Error::from(e).into())),
        };

        // set content-type
        self.set_header_if_none(header::CONTENT_TYPE, "application/json");

        Either::B(self.send_body(Body::Bytes(Bytes::from(body))))
    }

    /// Set a urlencoded body and generate `ClientRequest`
    ///
    /// `ClientRequestBuilder` can not be used after this call.
    pub fn send_form<T: Serialize>(
        &mut self,
        value: T,
    ) -> impl Future<
        Item = ClientResponse<impl Stream<Item = Bytes, Error = PayloadError>>,
        Error = SendRequestError,
    > {
        let body = match serde_urlencoded::to_string(&value) {
            Ok(body) => body,
            Err(e) => return Either::A(err(Error::from(e).into())),
        };

        // set content-type
        self.set_header_if_none(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        );

        Either::B(self.send_body(Body::Bytes(Bytes::from(body))))
    }

    /// Set an streaming body and generate `ClientRequest`.
    pub fn send_stream<S, E>(
        &mut self,
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
        &mut self,
    ) -> impl Future<
        Item = ClientResponse<impl Stream<Item = Bytes, Error = PayloadError>>,
        Error = SendRequestError,
    > {
        self.send_body(Body::Empty)
    }
}

impl std::ops::Deref for ClientRequest {
    type Target = RequestHead;

    fn deref(&self) -> &RequestHead {
        self.head.as_ref().expect("cannot reuse response builder")
    }
}

impl std::ops::DerefMut for ClientRequest {
    fn deref_mut(&mut self) -> &mut RequestHead {
        self.head.as_mut().expect("cannot reuse response builder")
    }
}

impl fmt::Debug for ClientRequest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let head = self.head.as_ref().expect("cannot reuse response builder");

        writeln!(
            f,
            "\nClientRequest {:?} {}:{}",
            head.version, head.method, head.uri
        )?;
        writeln!(f, "  headers:")?;
        for (key, val) in head.headers.iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

#[inline]
fn parts<'a>(
    parts: &'a mut Option<RequestHead>,
    err: &Option<HttpError>,
) -> Option<&'a mut RequestHead> {
    if err.is_some() {
        return None;
    }
    parts.as_mut()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test, Client};

    #[test]
    fn test_debug() {
        test::run_on(|| {
            let mut request = Client::new().get("/");
            request.header("x-test", "111");
            let repr = format!("{:?}", request);
            assert!(repr.contains("ClientRequest"));
            assert!(repr.contains("x-test"));
        })
    }

    #[test]
    fn test_client_header() {
        test::run_on(|| {
            let req = Client::build()
                .header(header::CONTENT_TYPE, "111")
                .finish()
                .get("/");

            assert_eq!(
                req.head
                    .as_ref()
                    .unwrap()
                    .headers
                    .get(header::CONTENT_TYPE)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "111"
            );
        })
    }

    #[test]
    fn test_client_header_override() {
        test::run_on(|| {
            let mut req = Client::build()
                .header(header::CONTENT_TYPE, "111")
                .finish()
                .get("/");
            req.set_header(header::CONTENT_TYPE, "222");

            assert_eq!(
                req.head
                    .as_ref()
                    .unwrap()
                    .headers
                    .get(header::CONTENT_TYPE)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "222"
            );
        })
    }

    #[test]
    fn client_basic_auth() {
        test::run_on(|| {
            let mut req = Client::new().get("/");
            req.basic_auth("username", Some("password"));
            assert_eq!(
                req.head
                    .as_ref()
                    .unwrap()
                    .headers
                    .get(header::AUTHORIZATION)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "Basic dXNlcm5hbWU6cGFzc3dvcmQ="
            );

            let mut req = Client::new().get("/");
            req.basic_auth("username", None);
            assert_eq!(
                req.head
                    .as_ref()
                    .unwrap()
                    .headers
                    .get(header::AUTHORIZATION)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "Basic dXNlcm5hbWU="
            );
        });
    }

    #[test]
    fn client_bearer_auth() {
        test::run_on(|| {
            let mut req = Client::new().get("/");
            req.bearer_auth("someS3cr3tAutht0k3n");
            assert_eq!(
                req.head
                    .as_ref()
                    .unwrap()
                    .headers
                    .get(header::AUTHORIZATION)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "Bearer someS3cr3tAutht0k3n"
            );
        })
    }
}
