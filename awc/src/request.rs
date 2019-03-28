use std::cell::RefCell;
use std::fmt;
use std::io::Write;
use std::rc::Rc;

use bytes::{BufMut, Bytes, BytesMut};
#[cfg(feature = "cookies")]
use cookie::{Cookie, CookieJar};
use futures::future::{err, Either};
use futures::{Future, Stream};
use serde::Serialize;
use serde_json;

use actix_http::body::{Body, BodyStream};
use actix_http::encoding::Decoder;
use actix_http::http::header::{self, ContentEncoding, Header, IntoHeaderValue};
use actix_http::http::{
    uri, ConnectionType, Error as HttpError, HeaderName, HeaderValue, HttpTryFrom,
    Method, Uri, Version,
};
use actix_http::{Error, Payload, RequestHead};

use crate::connect::Connect;
use crate::error::{InvalidUrl, PayloadError, SendRequestError};
use crate::response::ClientResponse;

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
    #[cfg(feature = "cookies")]
    cookies: Option<CookieJar>,
    default_headers: bool,
    response_decompress: bool,
    connector: Rc<RefCell<dyn Connect>>,
}

impl ClientRequest {
    /// Create new client request builder.
    pub(crate) fn new<U>(
        method: Method,
        uri: U,
        connector: Rc<RefCell<dyn Connect>>,
    ) -> Self
    where
        Uri: HttpTryFrom<U>,
    {
        let mut err = None;
        let mut head = RequestHead::default();
        head.method = method;

        match Uri::try_from(uri) {
            Ok(uri) => head.uri = uri,
            Err(e) => err = Some(e.into()),
        }

        ClientRequest {
            head,
            err,
            connector,
            #[cfg(feature = "cookies")]
            cookies: None,
            default_headers: true,
            response_decompress: true,
        }
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
                Ok(value) => {
                    self.head.headers.append(key, value);
                }
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
                Ok(value) => {
                    self.head.headers.insert(key, value);
                }
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
                        Ok(value) => {
                            self.head.headers.insert(key, value);
                        }
                        Err(e) => self.err = Some(e.into()),
                    }
                }
            }
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Close connection
    #[inline]
    pub fn close_connection(mut self) -> Self {
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
            Ok(value) => {
                let _ = self.head.headers.insert(header::CONTENT_TYPE, value);
            }
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
    pub fn basic_auth<U, P>(self, username: U, password: Option<P>) -> Self
    where
        U: fmt::Display,
        P: fmt::Display,
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
    pub fn bearer_auth<T>(self, token: T) -> Self
    where
        T: fmt::Display,
    {
        self.header(header::AUTHORIZATION, format!("Bearer {}", token))
    }

    #[cfg(feature = "cookies")]
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
    pub fn cookie<'c>(mut self, cookie: Cookie<'c>) -> Self {
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
    pub fn no_default_headers(mut self) -> Self {
        self.default_headers = false;
        self
    }

    /// Disable automatic decompress of response's body
    pub fn no_decompress(mut self) -> Self {
        self.response_decompress = false;
        self
    }

    /// This method calls provided closure with builder reference if
    /// value is `true`.
    pub fn if_true<F>(mut self, value: bool, f: F) -> Self
    where
        F: FnOnce(&mut ClientRequest),
    {
        if value {
            f(&mut self);
        }
        self
    }

    /// This method calls provided closure with builder reference if
    /// value is `Some`.
    pub fn if_some<T, F>(mut self, value: Option<T>, f: F) -> Self
    where
        F: FnOnce(T, &mut ClientRequest),
    {
        if let Some(val) = value {
            f(val, &mut self);
        }
        self
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

        // set default headers
        let slf = if self.default_headers {
            // set request host header
            if let Some(host) = self.head.uri.host() {
                if !self.head.headers.contains_key(header::HOST) {
                    let mut wrt = BytesMut::with_capacity(host.len() + 5).writer();

                    let _ = match self.head.uri.port_u16() {
                        None | Some(80) | Some(443) => write!(wrt, "{}", host),
                        Some(port) => write!(wrt, "{}:{}", host, port),
                    };

                    match wrt.get_mut().take().freeze().try_into() {
                        Ok(value) => {
                            self.head.headers.insert(header::HOST, value);
                        }
                        Err(e) => return Either::A(err(HttpError::from(e).into())),
                    }
                }
            }

            // user agent
            self.set_header_if_none(
                header::USER_AGENT,
                concat!("awc/", env!("CARGO_PKG_VERSION")),
            )
        } else {
            self
        };

        // enable br only for https
        let https = slf
            .head
            .uri
            .scheme_part()
            .map(|s| s == &uri::Scheme::HTTPS)
            .unwrap_or(true);

        #[cfg(any(
            feature = "brotli",
            feature = "flate2-zlib",
            feature = "flate2-rust"
        ))]
        let mut slf = {
            if https {
                slf.set_header_if_none(header::ACCEPT_ENCODING, HTTPS_ENCODING)
            } else {
                #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
                {
                    slf.set_header_if_none(header::ACCEPT_ENCODING, "gzip, deflate")
                }
                #[cfg(not(any(feature = "flate2-zlib", feature = "flate2-rust")))]
                slf
            }
        };

        #[allow(unused_mut)]
        let mut head = slf.head;

        #[cfg(feature = "cookies")]
        {
            use percent_encoding::{percent_encode, USERINFO_ENCODE_SET};
            use std::fmt::Write;

            // set cookies
            if let Some(ref mut jar) = slf.cookies {
                let mut cookie = String::new();
                for c in jar.delta() {
                    let name = percent_encode(c.name().as_bytes(), USERINFO_ENCODE_SET);
                    let value =
                        percent_encode(c.value().as_bytes(), USERINFO_ENCODE_SET);
                    let _ = write!(&mut cookie, "; {}={}", name, value);
                }
                head.headers.insert(
                    header::COOKIE,
                    HeaderValue::from_str(&cookie.as_str()[2..]).unwrap(),
                );
            }
        }

        let response_decompress = slf.response_decompress;

        let fut = slf
            .connector
            .borrow_mut()
            .send_request(head, body.into())
            .map(move |res| {
                res.map_body(|head, payload| {
                    if response_decompress {
                        Payload::Stream(Decoder::from_headers(&head.headers, payload))
                    } else {
                        Payload::Stream(Decoder::new(payload, ContentEncoding::Identity))
                    }
                })
            });
        Either::B(fut)
    }

    /// Set a JSON body and generate `ClientRequest`
    pub fn send_json<T: Serialize>(
        self,
        value: T,
    ) -> impl Future<
        Item = ClientResponse<impl Stream<Item = Bytes, Error = PayloadError>>,
        Error = SendRequestError,
    > {
        let body = match serde_json::to_string(&value) {
            Ok(body) => body,
            Err(e) => return Either::A(err(Error::from(e).into())),
        };
        // set content-type
        let slf = if !self.head.headers.contains_key(header::CONTENT_TYPE) {
            self.header(header::CONTENT_TYPE, "application/json")
        } else {
            self
        };

        Either::B(slf.send_body(Body::Bytes(Bytes::from(body))))
    }

    /// Set a urlencoded body and generate `ClientRequest`
    ///
    /// `ClientRequestBuilder` can not be used after this call.
    pub fn send_form<T: Serialize>(
        self,
        value: T,
    ) -> impl Future<
        Item = ClientResponse<impl Stream<Item = Bytes, Error = PayloadError>>,
        Error = SendRequestError,
    > {
        let body = match serde_urlencoded::to_string(&value) {
            Ok(body) => body,
            Err(e) => return Either::A(err(Error::from(e).into())),
        };

        let slf = if !self.head.headers.contains_key(header::CONTENT_TYPE) {
            self.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        } else {
            self
        };

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
