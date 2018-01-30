//! Http client request
#![allow(unused_imports, dead_code)]
use std::{fmt, io, str};
use std::rc::Rc;
use std::time::Duration;
use std::cell::UnsafeCell;

use base64;
use rand;
use cookie::Cookie;
use bytes::BytesMut;
use http::{HttpTryFrom, StatusCode, Error as HttpError};
use http::header::{self, HeaderName, HeaderValue};
use sha1::Sha1;
use futures::{Async, Future, Poll, Stream};
use futures::future::{Either, err as FutErr};
use tokio_core::net::TcpStream;

use actix::prelude::*;

use body::Binary;
use error::UrlParseError;
use server::shared::SharedBytes;

use server::{utils, IoStream};
use client::{ClientRequest, ClientRequestBuilder,
             HttpResponseParser, HttpResponseParserError, HttpClientWriter};
use client::{Connect, Connection, ClientConnector, ClientConnectorError};

use super::Message;
use super::proto::{CloseCode, OpCode};
use super::frame::Frame;

pub type WsClientFuture =
    Future<Item=(WsClientReader, WsClientWriter), Error=WsClientError>;


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
    Connector(ClientConnectorError),
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

impl From<ClientConnectorError> for WsClientError {
    fn from(err: ClientConnectorError) -> WsClientError {
        WsClientError::Connector(err)
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

/// WebSocket client
///
/// Example of WebSocket client usage is available in
/// [websocket example](
/// https://github.com/actix/actix-web/blob/master/examples/websocket/src/client.rs#L24)
pub struct WsClient {
    request: ClientRequestBuilder,
    err: Option<WsClientError>,
    http_err: Option<HttpError>,
    origin: Option<HeaderValue>,
    protocols: Option<String>,
    conn: Address<ClientConnector>,
}

impl WsClient {

    /// Create new websocket connection
    pub fn new<S: AsRef<str>>(uri: S) -> WsClient {
        WsClient::with_connector(uri, ClientConnector::from_registry())
    }

    /// Create new websocket connection with custom `ClientConnector`
    pub fn with_connector<S: AsRef<str>>(uri: S, conn: Address<ClientConnector>) -> WsClient {
        let mut cl = WsClient {
            request: ClientRequest::build(),
            err: None,
            http_err: None,
            origin: None,
            protocols: None,
            conn: conn,
        };
        cl.request.uri(uri.as_ref());
        cl
    }

    /// Set supported websocket protocols
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

    /// Set cookie for handshake request
    pub fn cookie<'c>(&mut self, cookie: Cookie<'c>) -> &mut Self {
        self.request.cookie(cookie);
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

    /// Set request header
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
        where HeaderName: HttpTryFrom<K>, HeaderValue: HttpTryFrom<V>
    {
        self.request.header(key, value);
        self
    }

    /// Connect to websocket server and do ws handshake
    pub fn connect(&mut self) -> Result<Box<WsClientFuture>, WsClientError> {
        if let Some(e) = self.err.take() {
            return Err(e)
        }
        if let Some(e) = self.http_err.take() {
            return Err(e.into())
        }

        // origin
        if let Some(origin) = self.origin.take() {
            self.request.set_header(header::ORIGIN, origin);
        }

        self.request.set_header(header::UPGRADE, "websocket");
        self.request.set_header(header::CONNECTION, "upgrade");
        self.request.set_header("SEC-WEBSOCKET-VERSION", "13");

        if let Some(protocols) = self.protocols.take() {
            self.request.set_header("SEC-WEBSOCKET-PROTOCOL", protocols.as_str());
        }
        let request = self.request.finish()?;

        if request.uri().host().is_none() {
            return Err(WsClientError::InvalidUrl)
        }
        if let Some(scheme) = request.uri().scheme_part() {
            if scheme != "http" && scheme != "https" && scheme != "ws" && scheme != "wss" {
                return Err(WsClientError::InvalidUrl);
            }
        } else {
            return Err(WsClientError::InvalidUrl);
        }

        // get connection and start handshake
        Ok(Box::new(
            self.conn.call_fut(Connect(request.uri().clone()))
                .map_err(|_| WsClientError::Disconnected)
                .and_then(|res| match res {
                    Ok(stream) => Either::A(WsHandshake::new(stream, request)),
                    Err(err) => Either::B(FutErr(err.into())),
                })
        ))
    }
}

struct WsInner {
    conn: Connection,
    writer: HttpClientWriter,
    parser: HttpResponseParser,
    parser_buf: BytesMut,
    closed: bool,
    error_sent: bool,
}

struct WsHandshake {
    inner: Option<WsInner>,
    request: ClientRequest,
    sent: bool,
    key: String,
}

impl WsHandshake {
    fn new(conn: Connection, mut request: ClientRequest) -> WsHandshake {
        // Generate a random key for the `Sec-WebSocket-Key` header.
        // a base64-encoded (see Section 4 of [RFC4648]) value that,
        // when decoded, is 16 bytes in length (RFC 6455)
        let sec_key: [u8; 16] = rand::random();
        let key = base64::encode(&sec_key);

        request.headers_mut().insert(
            HeaderName::try_from("SEC-WEBSOCKET-KEY").unwrap(),
            HeaderValue::try_from(key.as_str()).unwrap());

        let inner = WsInner {
            conn: conn,
            writer: HttpClientWriter::new(SharedBytes::default()),
            parser: HttpResponseParser::default(),
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

impl Future for WsHandshake {
    type Item = (WsClientReader, WsClientWriter);
    type Error = WsClientError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut inner = self.inner.take().unwrap();

        if !self.sent {
            self.sent = true;
            inner.writer.start(&mut self.request);
        }
        if let Err(err) = inner.writer.poll_completed(&mut inner.conn, false) {
            return Err(err.into())
        }

        match inner.parser.parse(&mut inner.conn, &mut inner.parser_buf) {
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
                    // field is constructed by concatenating /key/
                    // with the string "258EAFA5-E914-47DA-95CA-C5AB0DC85B11" (RFC 6455)
                    const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
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
                    (WsClientReader{inner: Rc::clone(&inner)},
                     WsClientWriter{inner: inner})))
            },
            Ok(Async::NotReady) => {
                self.inner = Some(inner);
                Ok(Async::NotReady)
            },
            Err(err) => Err(err.into())
        }
    }
}


struct Inner {
    inner: WsInner,
}

pub struct WsClientReader {
    inner: Rc<UnsafeCell<Inner>>
}

impl fmt::Debug for WsClientReader {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "WsClientReader()")
    }
}

impl WsClientReader {
    #[inline]
    fn as_mut(&mut self) -> &mut Inner {
        unsafe{ &mut *self.inner.get() }
    }
}

impl Stream for WsClientReader {
    type Item = Message;
    type Error = WsClientError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let inner = self.as_mut();
        let mut done = false;

        match utils::read_from_io(&mut inner.inner.conn, &mut inner.inner.parser_buf) {
            Ok(Async::Ready(0)) => {
                done = true;
                inner.inner.closed = true;
            },
            Ok(Async::Ready(_)) | Ok(Async::NotReady) => (),
            Err(err) =>
                return Err(err.into())
        }

        // write
        let _ = inner.inner.writer.poll_completed(&mut inner.inner.conn, false);

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

pub struct WsClientWriter {
    inner: Rc<UnsafeCell<Inner>>
}

impl WsClientWriter {
    #[inline]
    fn as_mut(&mut self) -> &mut Inner {
        unsafe{ &mut *self.inner.get() }
    }
}

impl WsClientWriter {

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
