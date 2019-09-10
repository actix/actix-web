use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};
use std::{fmt, net};

use bytes::{BufMut, Bytes, BytesMut};
use futures::{Async, Future, Poll, Stream, try_ready};
use percent_encoding::percent_encode;
use serde::Serialize;
use serde_json;
use tokio_timer::Delay;
use derive_more::From;

use actix_http::body::{Body, BodyStream};
use actix_http::cookie::{Cookie, CookieJar, USERINFO};
use actix_http::encoding::Decoder;
use actix_http::http::header::{self, ContentEncoding, Header, IntoHeaderValue};
use actix_http::http::{
    uri, ConnectionType, Error as HttpError, HeaderMap, HeaderName, HeaderValue,
    HttpTryFrom, Method, Uri, Version,
};
use actix_http::{Error, Payload, PayloadStream, RequestHead};

use crate::error::{InvalidUrl, SendRequestError, FreezeRequestError};
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

    /// Get HTTP URI of request
    pub fn get_uri(&self) -> &Uri {
        &self.head.uri
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

    /// Get HTTP method of this request
    pub fn get_method(&self) -> &Method {
        &self.head.method
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

    pub fn freeze(self) -> Result<FrozenClientRequest, FreezeRequestError> {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(e) => return Err(e.into()),
        };

        let request = FrozenClientRequest {
            head: Rc::new(slf.head),
            addr: slf.addr,
            response_decompress: slf.response_decompress,
            timeout: slf.timeout,
            config: slf.config,
        };

        Ok(request)
    }

    /// Complete request construction and send body.
    pub fn send_body<B>(
        self,
        body: B,
    ) -> SendBody
    where
        B: Into<Body>,
    {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(e) => return e.into(),
        };

        RequestSender::Owned(slf.head)
            .send_body(slf.addr, slf.response_decompress, slf.timeout, slf.config.as_ref(), body)
    }

    /// Set a JSON body and generate `ClientRequest`
    pub fn send_json<T: Serialize>(
        self,
        value: &T,
    ) -> SendBody
    {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(e) => return e.into(),
        };

        RequestSender::Owned(slf.head)
            .send_json(slf.addr, slf.response_decompress, slf.timeout, slf.config.as_ref(), value)
    }

    /// Set a urlencoded body and generate `ClientRequest`
    ///
    /// `ClientRequestBuilder` can not be used after this call.
    pub fn send_form<T: Serialize>(
        self,
        value: &T,
    ) -> SendBody
    {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(e) => return e.into(),
        };

        RequestSender::Owned(slf.head)
            .send_form(slf.addr, slf.response_decompress, slf.timeout, slf.config.as_ref(), value)
    }

    /// Set an streaming body and generate `ClientRequest`.
    pub fn send_stream<S, E>(
        self,
        stream: S,
    ) -> SendBody
    where
        S: Stream<Item = Bytes, Error = E> + 'static,
        E: Into<Error> + 'static,
    {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(e) => return e.into(),
        };

        RequestSender::Owned(slf.head)
            .send_stream(slf.addr, slf.response_decompress, slf.timeout, slf.config.as_ref(), stream)
    }

    /// Set an empty body and generate `ClientRequest`.
    pub fn send(
        self,
    ) -> SendBody
    {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(e) => return e.into(),
        };

        RequestSender::Owned(slf.head)
            .send(slf.addr, slf.response_decompress, slf.timeout, slf.config.as_ref())
    }

    fn prep_for_sending(mut self) -> Result<Self, PrepForSendingError> {
        if let Some(e) = self.err {
            return Err(e.into());
        }

        // validate uri
        let uri = &self.head.uri;
        if uri.host().is_none() {
            return Err(InvalidUrl::MissingHost.into());
        } else if uri.scheme_part().is_none() {
            return Err(InvalidUrl::MissingScheme.into());
        } else if let Some(scheme) = uri.scheme_part() {
            match scheme.as_str() {
                "http" | "ws" | "https" | "wss" => (),
                _ => return Err(InvalidUrl::UnknownScheme.into()),
            }
        } else {
            return Err(InvalidUrl::UnknownScheme.into());
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

        Ok(slf)
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

#[derive(Clone)]
pub struct FrozenClientRequest {
    pub(crate) head: Rc<RequestHead>,
    pub(crate) addr: Option<net::SocketAddr>,
    pub(crate) response_decompress: bool,
    pub(crate) timeout: Option<Duration>,
    pub(crate) config: Rc<ClientConfig>,
}

impl FrozenClientRequest {
    /// Get HTTP URI of request
    pub fn get_uri(&self) -> &Uri {
        &self.head.uri
    }

    /// Get HTTP method of this request
    pub fn get_method(&self) -> &Method {
        &self.head.method
    }

    /// Returns request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    /// Send a body.
    pub fn send_body<B>(
        &self,
        body: B,
    ) -> SendBody
    where
        B: Into<Body>,
    {
        RequestSender::Rc(self.head.clone(), None)
            .send_body(self.addr, self.response_decompress, self.timeout, self.config.as_ref(), body)
    }

    /// Send a json body.
    pub fn send_json<T: Serialize>(
        &self,
        value: &T,
    ) -> SendBody
    {
        RequestSender::Rc(self.head.clone(), None)
            .send_json(self.addr, self.response_decompress, self.timeout, self.config.as_ref(), value)
    }

    /// Send an urlencoded body.
    pub fn send_form<T: Serialize>(
        &self,
        value: &T,
    ) -> SendBody
    {
        RequestSender::Rc(self.head.clone(), None)
            .send_form(self.addr, self.response_decompress, self.timeout, self.config.as_ref(), value)
    }

    /// Send a streaming body.
    pub fn send_stream<S, E>(
        &self,
        stream: S,
    ) -> SendBody
    where
        S: Stream<Item = Bytes, Error = E> + 'static,
        E: Into<Error> + 'static,
    {
        RequestSender::Rc(self.head.clone(), None)
            .send_stream(self.addr, self.response_decompress, self.timeout, self.config.as_ref(), stream)
    }

    /// Send an empty body.
    pub fn send(
        &self,
    ) -> SendBody
    {
        RequestSender::Rc(self.head.clone(), None)
            .send(self.addr, self.response_decompress, self.timeout, self.config.as_ref())
    }

    /// Create a `FrozenSendBuilder` with extra headers
    pub fn extra_headers(&self, extra_headers: HeaderMap) -> FrozenSendBuilder {
        FrozenSendBuilder::new(self.clone(), extra_headers)
    }

    /// Create a `FrozenSendBuilder` with an extra header
    pub fn extra_header<K, V>(&self, key: K, value: V) -> FrozenSendBuilder
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        self.extra_headers(HeaderMap::new()).extra_header(key, value)
    }
}

pub struct FrozenSendBuilder {
    req: FrozenClientRequest,
    extra_headers: HeaderMap,
    err: Option<HttpError>,
}

impl FrozenSendBuilder {
    pub(crate) fn new(req: FrozenClientRequest, extra_headers: HeaderMap) -> Self {
        Self {
            req,
            extra_headers,
            err: None,
        }
    }

    /// Insert a header, it overrides existing header in `FrozenClientRequest`.
    pub fn extra_header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match value.try_into() {
                Ok(value) => self.extra_headers.insert(key, value),
                Err(e) => self.err = Some(e.into()),
            },
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Complete request construction and send a body.
    pub fn send_body<B>(
        self,
        body: B,
    ) -> SendBody
    where
        B: Into<Body>,
    {
        if let Some(e) = self.err {
            return e.into()
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers))
            .send_body(self.req.addr, self.req.response_decompress, self.req.timeout, self.req.config.as_ref(), body)
    }

    /// Complete request construction and send a json body.
    pub fn send_json<T: Serialize>(
        self,
        value: &T,
    ) -> SendBody
    {
        if let Some(e) = self.err {
            return e.into()
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers))
            .send_json(self.req.addr, self.req.response_decompress, self.req.timeout, self.req.config.as_ref(), value)
    }

    /// Complete request construction and send an urlencoded body.
    pub fn send_form<T: Serialize>(
        self,
        value: &T,
    ) -> SendBody
    {
        if let Some(e) = self.err {
            return e.into()
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers))
            .send_form(self.req.addr, self.req.response_decompress, self.req.timeout, self.req.config.as_ref(), value)
    }

    /// Complete request construction and send a streaming body.
    pub fn send_stream<S, E>(
        self,
        stream: S,
    ) -> SendBody
    where
        S: Stream<Item = Bytes, Error = E> + 'static,
        E: Into<Error> + 'static,
    {
        if let Some(e) = self.err {
            return e.into()
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers))
            .send_stream(self.req.addr, self.req.response_decompress, self.req.timeout, self.req.config.as_ref(), stream)
    }

    /// Complete request construction and send an empty body.
    pub fn send(
        self,
    ) -> SendBody
    {
        if let Some(e) = self.err {
            return e.into()
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers))
            .send(self.req.addr, self.req.response_decompress, self.req.timeout, self.req.config.as_ref())
    }
}

#[derive(Debug, From)]
enum PrepForSendingError {
    Url(InvalidUrl),
    Http(HttpError),
}

impl Into<FreezeRequestError> for PrepForSendingError {
    fn into(self) -> FreezeRequestError {
        match self {
            PrepForSendingError::Url(e) => FreezeRequestError::Url(e),
            PrepForSendingError::Http(e) => FreezeRequestError::Http(e),
        }
    }
}

impl Into<SendRequestError> for PrepForSendingError {
    fn into(self) -> SendRequestError {
        match self {
            PrepForSendingError::Url(e) => SendRequestError::Url(e),
            PrepForSendingError::Http(e) => SendRequestError::Http(e),
        }
    }
}

pub enum SendBody
{
    Fut(Box<dyn Future<Item = ClientResponse, Error = SendRequestError>>, Option<Delay>, bool),
    Err(Option<SendRequestError>),
}

impl SendBody
{
    pub fn new(
        send: Box<dyn Future<Item = ClientResponse, Error = SendRequestError>>,
        response_decompress: bool,
        timeout: Option<Duration>,
    ) -> SendBody
    {
        let delay = timeout.map(|t| Delay::new(Instant::now() + t));
        SendBody::Fut(send, delay, response_decompress)
    }
}

impl Future for SendBody
{
    type Item = ClientResponse<Decoder<Payload<PayloadStream>>>;
    type Error = SendRequestError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self {
            SendBody::Fut(send, delay, response_decompress) => {
                if delay.is_some() {
                    match delay.poll() {
                        Ok(Async::NotReady) => (),
                        _ => return Err(SendRequestError::Timeout),
                    }
                }

                let res = try_ready!(send.poll())
                    .map_body(|head, payload| {
                        if *response_decompress {
                            Payload::Stream(Decoder::from_headers(payload, &head.headers))
                        } else {
                            Payload::Stream(Decoder::new(payload, ContentEncoding::Identity))
                        }
                    });

                Ok(Async::Ready(res))
            },
            SendBody::Err(ref mut e) => {
                match e.take() {
                    Some(e) => Err(e.into()),
                    None => panic!("Attempting to call completed future"),
                }
            }
        }
    }
}


impl From<SendRequestError> for SendBody
{
    fn from(e: SendRequestError) -> Self {
        SendBody::Err(Some(e))
    }
}

impl From<Error> for SendBody
{
    fn from(e: Error) -> Self {
        SendBody::Err(Some(e.into()))
    }
}

impl From<HttpError> for SendBody
{
    fn from(e: HttpError) -> Self {
        SendBody::Err(Some(e.into()))
    }
}

impl From<PrepForSendingError> for SendBody
{
    fn from(e: PrepForSendingError) -> Self {
        SendBody::Err(Some(e.into()))
    }
}

#[derive(Debug)]
enum RequestSender {
    Owned(RequestHead),
    Rc(Rc<RequestHead>, Option<HeaderMap>),
}

impl RequestSender {
    pub fn send_body<B>(
        self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
        body: B,
    ) -> SendBody
    where
        B: Into<Body>,
    {
        let mut connector = config.connector.borrow_mut();

        let fut = match self {
            RequestSender::Owned(head) => connector.send_request(head, body.into(), addr),
            RequestSender::Rc(head, extra_headers) => connector.send_request_extra(head, extra_headers, body.into(), addr),
        };

        SendBody::new(fut, response_decompress, timeout.or_else(|| config.timeout.clone()))
    }

    pub fn send_json<T: Serialize>(
        mut self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
        value: &T,
    ) -> SendBody
    {
        let body = match serde_json::to_string(value) {
            Ok(body) => body,
            Err(e) => return Error::from(e).into(),
        };

        if let Err(e) = self.set_header_if_none(header::CONTENT_TYPE, "application/json") {
            return e.into();
        }

        self.send_body(addr, response_decompress, timeout, config, Body::Bytes(Bytes::from(body)))
    }

    pub fn send_form<T: Serialize>(
        mut self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
        value: &T,
    ) -> SendBody
    {
        let body = match serde_urlencoded::to_string(value) {
            Ok(body) => body,
            Err(e) => return Error::from(e).into(),
        };

        // set content-type
        if let Err(e) = self.set_header_if_none(header::CONTENT_TYPE, "application/x-www-form-urlencoded") {
            return e.into();
        }

        self.send_body(addr, response_decompress, timeout, config, Body::Bytes(Bytes::from(body)))
    }

    pub fn send_stream<S, E>(
        self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
        stream: S,
    ) -> SendBody
    where
        S: Stream<Item = Bytes, Error = E> + 'static,
        E: Into<Error> + 'static,
    {
        self.send_body(addr, response_decompress, timeout, config, Body::from_message(BodyStream::new(stream)))
    }

    pub fn send(
        self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
    ) -> SendBody
    {
        self.send_body(addr, response_decompress, timeout, config, Body::Empty)
    }

    fn set_header_if_none<V>(&mut self, key: HeaderName, value: V) -> Result<(), HttpError>
    where
        V: IntoHeaderValue,
    {
        match self {
            RequestSender::Owned(head) => {
                if !head.headers.contains_key(&key) {
                    match value.try_into() {
                        Ok(value) => head.headers.insert(key, value),
                        Err(e) => return Err(e.into()),
                    }
                }
            },
            RequestSender::Rc(head, extra_headers) => {
                if !head.headers.contains_key(&key) && !extra_headers.iter().any(|h| h.contains_key(&key)) {
                    match value.try_into(){
                        Ok(v) => {
                            let h = extra_headers.get_or_insert(HeaderMap::new());
                            h.insert(key, v)
                        },
                        Err(e) => return Err(e.into()),
                    };
                }
            }
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
