//! Http client request
use std::{fmt, io, str};
use std::rc::Rc;
use std::time::Duration;
use std::cell::UnsafeCell;

use base64;
use rand;
use cookie::{Cookie, CookieJar};
use bytes::BytesMut;
use http::{Method, Version, HeaderMap, HttpTryFrom, StatusCode, Error as HttpError};
use http::header::{self, HeaderName, HeaderValue};
use url::Url;
use sha1::Sha1;
use futures::{Async, Future, Poll, Stream};
// use futures::unsync::oneshot;
use tokio_core::net::TcpStream;

use body::{Body, Binary};
use error::UrlParseError;
use headers::ContentEncoding;
use server::shared::SharedBytes;

use server::{utils, IoStream};
use client::{HttpResponseParser, HttpResponseParserError};

use super::Message;
use super::proto::{CloseCode, OpCode};
use super::frame::Frame;
use super::writer::Writer;
use super::connect::{TcpConnector, TcpConnectorError};

/// Websockt client error
#[derive(Fail, Debug)]
pub enum WsClientError {
    #[fail(display="Invalid url")]
    InvalidUrl,
    #[fail(display="Invalid response status")]
    InvalidResponseStatus,
    #[fail(display="Invalid upgrade header")]
    InvalidUpgradeHeader,
    #[fail(display="Invalid connection header")]
    InvalidConnectionHeader,
    #[fail(display="Invalid challenge response")]
    InvalidChallengeResponse,
    #[fail(display="Http parsing error")]
    Http(HttpError),
    #[fail(display="Url parsing error")]
    Url(UrlParseError),
    #[fail(display="Response parsing error")]
    ResponseParseError(HttpResponseParserError),
    #[fail(display="{}", _0)]
    Connection(TcpConnectorError),
    #[fail(display="{}", _0)]
    Io(io::Error),
    #[fail(display="Disconnected")]
    Disconnected,
}

impl From<HttpError> for WsClientError {
    fn from(err: HttpError) -> WsClientError {
        WsClientError::Http(err)
    }
}

impl From<UrlParseError> for WsClientError {
    fn from(err: UrlParseError) -> WsClientError {
        WsClientError::Url(err)
    }
}

impl From<TcpConnectorError> for WsClientError {
    fn from(err: TcpConnectorError) -> WsClientError {
        WsClientError::Connection(err)
    }
}

impl From<io::Error> for WsClientError {
    fn from(err: io::Error) -> WsClientError {
        WsClientError::Io(err)
    }
}

impl From<HttpResponseParserError> for WsClientError {
    fn from(err: HttpResponseParserError) -> WsClientError {
        WsClientError::ResponseParseError(err)
    }
}

type WsFuture<T> = Future<Item=(WsReader<T>, WsWriter<T>), Error=WsClientError>;

/// Websockt client
pub struct WsClient {
    request: Option<ClientRequest>,
    err: Option<WsClientError>,
    http_err: Option<HttpError>,
    cookies: Option<CookieJar>,
    origin: Option<HeaderValue>,
    protocols: Option<String>,
}

impl WsClient {

    pub fn new<S: AsRef<str>>(url: S) -> WsClient {
        let mut cl = WsClient {
            request: None,
            err: None,
            http_err: None,
            cookies: None,
            origin: None,
            protocols: None };

        match Url::parse(url.as_ref()) {
            Ok(url) => {
                if url.scheme() != "http" && url.scheme() != "https" &&
                    url.scheme() != "ws" && url.scheme() != "wss" || !url.has_host() {
                        cl.err = Some(WsClientError::InvalidUrl);
                } else {
                    cl.request = Some(ClientRequest::new(Method::GET, url));
                }
            },
            Err(err) => cl.err = Some(err.into()),
        }
        cl
    }

    pub fn protocols<U, V>(&mut self, protos: U) -> &mut Self
        where U: IntoIterator<Item=V> + 'static,
              V: AsRef<str>
    {
        let mut protos = protos.into_iter()
            .fold(String::new(), |acc, s| {acc + s.as_ref() + ","});
        protos.pop();
        self.protocols = Some(protos);
        self
    }

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

    /// Set request Origin
    pub fn origin<V>(&mut self, origin: V) -> &mut Self
        where HeaderValue: HttpTryFrom<V>
    {
        match HeaderValue::try_from(origin) {
            Ok(value) => self.origin = Some(value),
            Err(e) => self.http_err = Some(e.into()),
        }
        self
    }

    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
        where HeaderName: HttpTryFrom<K>,
              HeaderValue: HttpTryFrom<V>
    {
        if let Some(parts) = parts(&mut self.request, &self.err, &self.http_err) {
            match HeaderName::try_from(key) {
                Ok(key) => {
                    match HeaderValue::try_from(value) {
                        Ok(value) => { parts.headers.append(key, value); }
                        Err(e) => self.http_err = Some(e.into()),
                    }
                },
                Err(e) => self.http_err = Some(e.into()),
            };
        }
        self
    }

    pub fn connect(&mut self) -> Result<Box<WsFuture<TcpStream>>, WsClientError> {
        if let Some(e) = self.err.take() {
            return Err(e)
        }
        if let Some(e) = self.http_err.take() {
            return Err(e.into())
        }
        let mut request = self.request.take().expect("cannot reuse request builder");

        // headers
        if let Some(ref jar) = self.cookies {
            for cookie in jar.delta() {
                request.headers.append(
                    header::SET_COOKIE,
                    HeaderValue::from_str(&cookie.to_string()).map_err(HttpError::from)?);
            }
        }

        // origin
        if let Some(origin) = self.origin.take() {
            request.headers.insert(header::ORIGIN, origin);
        }

        request.headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        request.headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
        request.headers.insert(
            HeaderName::try_from("SEC-WEBSOCKET-VERSION").unwrap(),
            HeaderValue::from_static("13"));

        if let Some(protocols) = self.protocols.take() {
            request.headers.insert(
                HeaderName::try_from("SEC-WEBSOCKET-PROTOCOL").unwrap(),
                HeaderValue::try_from(protocols.as_str()).unwrap());
        }

        let connect = TcpConnector::new(
            request.url.host_str().unwrap(),
            request.url.port().unwrap_or(80), Duration::from_secs(5));

        Ok(Box::new(
            connect
                .from_err()
                .and_then(move |stream| WsHandshake::new(stream, request))))
    }
}

#[inline]
fn parts<'a>(parts: &'a mut Option<ClientRequest>,
             err: &Option<WsClientError>,
             http_err: &Option<HttpError>) -> Option<&'a mut ClientRequest>
{
    if err.is_some() || http_err.is_some() {
        return None
    }
    parts.as_mut()
}

pub(crate) struct ClientRequest {
    pub url: Url,
    pub method: Method,
    pub version: Version,
    pub headers: HeaderMap,
    pub body: Body,
    pub chunked: Option<bool>,
    pub encoding: ContentEncoding,
}

impl ClientRequest {

    #[inline]
    fn new(method: Method, url: Url) -> ClientRequest {
        ClientRequest {
            url: url,
            method: method,
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(16),
            body: Body::Empty,
            chunked: None,
            encoding: ContentEncoding::Auto,
        }
    }
}

impl fmt::Debug for ClientRequest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(f, "\nClientRequest {:?} {}:{}\n",
                         self.version, self.method, self.url);
        let _ = write!(f, "  headers:\n");
        for key in self.headers.keys() {
            let vals: Vec<_> = self.headers.get_all(key).iter().collect();
            if vals.len() > 1 {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals);
            } else {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals[0]);
            }
        }
        res
    }
}

struct WsInner<T> {
    stream: T,
    writer: Writer,
    parser: HttpResponseParser,
    parser_buf: BytesMut,
    closed: bool,
    error_sent: bool,
}

struct WsHandshake<T> {
    inner: Option<WsInner<T>>,
    request: ClientRequest,
    sent: bool,
    key: String,
}

impl<T: IoStream> WsHandshake<T> {
    fn new(stream: T, mut request: ClientRequest) -> WsHandshake<T> {
        // Generate a random key for the `Sec-WebSocket-Key` header.
        // a base64-encoded (see Section 4 of [RFC4648]) value that,
        // when decoded, is 16 bytes in length (RFC 6455)
        let sec_key: [u8; 16] = rand::random();
        let key = base64::encode(&sec_key);

        request.headers.insert(
            HeaderName::try_from("SEC-WEBSOCKET-KEY").unwrap(),
            HeaderValue::try_from(key.as_str()).unwrap());

        let inner = WsInner {
            stream: stream,
            writer: Writer::new(SharedBytes::default()),
            parser: HttpResponseParser::new(),
            parser_buf: BytesMut::new(),
            closed: false,
            error_sent: false,
        };

        WsHandshake {
            key: key,
            inner: Some(inner),
            request: request,
            sent: false,
        }
    }
}

impl<T: IoStream> Future for WsHandshake<T> {
    type Item = (WsReader<T>, WsWriter<T>);
    type Error = WsClientError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut inner = self.inner.take().unwrap();

        if !self.sent {
            self.sent = true;
            inner.writer.start(&mut self.request);
        }
        if let Err(err) = inner.writer.poll_completed(&mut inner.stream, false) {
            return Err(err.into())
        }

        match inner.parser.parse(&mut inner.stream, &mut inner.parser_buf) {
            Ok(Async::Ready(resp)) => {
                // verify response
                if resp.status() != StatusCode::SWITCHING_PROTOCOLS {
                    return Err(WsClientError::InvalidResponseStatus)
                }
                // Check for "UPGRADE" to websocket header
                let has_hdr = if let Some(hdr) = resp.headers().get(header::UPGRADE) {
                    if let Ok(s) = hdr.to_str() {
                        s.to_lowercase().contains("websocket")
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !has_hdr {
                    return Err(WsClientError::InvalidUpgradeHeader)
                }
                // Check for "CONNECTION" header
                let has_hdr = if let Some(conn) = resp.headers().get(header::CONNECTION) {
                    if let Ok(s) = conn.to_str() {
                        s.to_lowercase().contains("upgrade")
                    } else { false }
                } else { false };
                if !has_hdr {
                    return Err(WsClientError::InvalidConnectionHeader)
                }

                let match_key = if let Some(key) = resp.headers().get(
                    HeaderName::try_from("SEC-WEBSOCKET-ACCEPT").unwrap())
                {
                    // ... field is constructed by concatenating /key/ ...
                    // ... with the string "258EAFA5-E914-47DA-95CA-C5AB0DC85B11" (RFC 6455)
                    const WS_GUID: &'static [u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
                    let mut sha1 = Sha1::new();
                    sha1.update(self.key.as_ref());
                    sha1.update(WS_GUID);
                    key.as_bytes() == base64::encode(&sha1.digest().bytes()).as_bytes()
                } else {
                    false
                };
                if !match_key {
                    return Err(WsClientError::InvalidChallengeResponse)
                }

                let inner = Rc::new(UnsafeCell::new(Inner{inner: inner}));
                Ok(Async::Ready(
                    (WsReader{inner: Rc::clone(&inner)},
                     WsWriter{inner: inner})))
            },
            Ok(Async::NotReady) => {
                self.inner = Some(inner);
                Ok(Async::NotReady)
            },
            Err(err) => Err(err.into())
        }
    }
}


struct Inner<T> {
    inner: WsInner<T>,
}

pub struct WsReader<T> {
    inner: Rc<UnsafeCell<Inner<T>>>
}

impl<T> WsReader<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut Inner<T> {
        unsafe{ &mut *self.inner.get() }
    }
}

impl<T: IoStream> Stream for WsReader<T> {
    type Item = Message;
    type Error = WsClientError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let inner = self.as_mut();
        let mut done = false;

        match utils::read_from_io(&mut inner.inner.stream, &mut inner.inner.parser_buf) {
            Ok(Async::Ready(0)) => {
                done = true;
                inner.inner.closed = true;
            },
            Ok(Async::Ready(_)) | Ok(Async::NotReady) => (),
            Err(err) =>
                return Err(err.into())
        }

        // write
        let _ = inner.inner.writer.poll_completed(&mut inner.inner.stream, false);

        // read
        match Frame::parse(&mut inner.inner.parser_buf) {
            Ok(Some(frame)) => {
                // trace!("WsFrame {}", frame);
                let (_finished, opcode, payload) = frame.unpack();

                match opcode {
                    OpCode::Continue => unimplemented!(),
                    OpCode::Bad =>
                        Ok(Async::Ready(Some(Message::Error))),
                    OpCode::Close => {
                        inner.inner.closed = true;
                        inner.inner.error_sent = true;
                        Ok(Async::Ready(Some(Message::Closed)))
                    },
                    OpCode::Ping =>
                        Ok(Async::Ready(Some(
                            Message::Ping(
                                String::from_utf8_lossy(payload.as_ref()).into())))),
                    OpCode::Pong =>
                        Ok(Async::Ready(Some(
                            Message::Pong(
                                String::from_utf8_lossy(payload.as_ref()).into())))),
                    OpCode::Binary =>
                        Ok(Async::Ready(Some(Message::Binary(payload)))),
                    OpCode::Text => {
                        let tmp = Vec::from(payload.as_ref());
                        match String::from_utf8(tmp) {
                            Ok(s) =>
                                Ok(Async::Ready(Some(Message::Text(s)))),
                            Err(_) =>
                                Ok(Async::Ready(Some(Message::Error))),
                        }
                    }
                }
            }
            Ok(None) => {
                if done {
                    Ok(Async::Ready(None))
                } else if inner.inner.closed {
                    if !inner.inner.error_sent {
                        inner.inner.error_sent = true;
                        Ok(Async::Ready(Some(Message::Closed)))
                    } else {
                        Ok(Async::Ready(None))
                    }
                } else {
                    Ok(Async::NotReady)
                }
            },
            Err(err) => {
                inner.inner.closed = true;
                inner.inner.error_sent = true;
                Err(err.into())
            }
        }
    }
}

pub struct WsWriter<T> {
    inner: Rc<UnsafeCell<Inner<T>>>
}

impl<T: IoStream> WsWriter<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut Inner<T> {
        unsafe{ &mut *self.inner.get() }
    }
}

impl<T: IoStream> WsWriter<T> {

    /// Write payload
    #[inline]
    fn write<B: Into<Binary>>(&mut self, data: B) {
        if !self.as_mut().inner.closed {
            let _ = self.as_mut().inner.writer.write(&data.into());
        } else {
            warn!("Trying to write to disconnected response");
        }
    }

    /// Send text frame
    pub fn text(&mut self, text: &str) {
        let mut frame = Frame::message(Vec::from(text), OpCode::Text, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        self.write(buf);
    }

    /// Send binary frame
    pub fn binary<B: Into<Binary>>(&mut self, data: B) {
        let mut frame = Frame::message(data, OpCode::Binary, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        self.write(buf);
    }

    /// Send ping frame
    pub fn ping(&mut self, message: &str) {
        let mut frame = Frame::message(Vec::from(message), OpCode::Ping, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        self.write(buf);
    }

    /// Send pong frame
    pub fn pong(&mut self, message: &str) {
        let mut frame = Frame::message(Vec::from(message), OpCode::Pong, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        self.write(buf);
    }

    /// Send close frame
    pub fn close(&mut self, code: CloseCode, reason: &str) {
        let mut frame = Frame::close(code, reason);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();
        self.write(buf);
    }
}
