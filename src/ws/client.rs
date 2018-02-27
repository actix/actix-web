//! Http client request
use std::{fmt, io, str};
use std::rc::Rc;
use std::cell::UnsafeCell;

use base64;
use rand;
use bytes::Bytes;
use cookie::Cookie;
use http::{HttpTryFrom, StatusCode, Error as HttpError};
use http::header::{self, HeaderName, HeaderValue};
use sha1::Sha1;
use futures::{Async, Future, Poll, Stream};
use futures::unsync::mpsc::{unbounded, UnboundedSender};
use byteorder::{ByteOrder, NetworkEndian};

use actix::prelude::*;

use body::{Body, Binary};
use error::UrlParseError;
use payload::PayloadHelper;
use httpmessage::HttpMessage;

use client::{ClientRequest, ClientRequestBuilder, ClientResponse,
             ClientConnector, SendRequest, SendRequestError,
             HttpResponseParserError};

use super::{Message, WsError};
use super::frame::Frame;
use super::proto::{CloseCode, OpCode};


/// Websocket client error
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
    SendRequest(SendRequestError),
    #[fail(display="{}", _0)]
    Protocol(#[cause] WsError),
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

impl From<SendRequestError> for WsClientError {
    fn from(err: SendRequestError) -> WsClientError {
        WsClientError::SendRequest(err)
    }
}

impl From<WsError> for WsClientError {
    fn from(err: WsError) -> WsClientError {
        WsClientError::Protocol(err)
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

/// `WebSocket` client
///
/// Example of `WebSocket` client usage is available in
/// [websocket example](
/// https://github.com/actix/actix-web/blob/master/examples/websocket/src/client.rs#L24)
pub struct WsClient {
    request: ClientRequestBuilder,
    err: Option<WsClientError>,
    http_err: Option<HttpError>,
    origin: Option<HeaderValue>,
    protocols: Option<String>,
    conn: Addr<Unsync, ClientConnector>,
    max_size: usize,
}

impl WsClient {

    /// Create new websocket connection
    pub fn new<S: AsRef<str>>(uri: S) -> WsClient {
        WsClient::with_connector(uri, ClientConnector::from_registry())
    }

    /// Create new websocket connection with custom `ClientConnector`
    pub fn with_connector<S: AsRef<str>>(uri: S, conn: Addr<Unsync, ClientConnector>) -> WsClient {
        let mut cl = WsClient {
            request: ClientRequest::build(),
            err: None,
            http_err: None,
            origin: None,
            protocols: None,
            max_size: 65_536,
            conn,
        };
        cl.request.uri(uri.as_ref());
        cl
    }

    /// Set supported websocket protocols
    pub fn protocols<U, V>(mut self, protos: U) -> Self
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
    pub fn cookie(mut self, cookie: Cookie) -> Self {
        self.request.cookie(cookie);
        self
    }

    /// Set request Origin
    pub fn origin<V>(mut self, origin: V) -> Self
        where HeaderValue: HttpTryFrom<V>
    {
        match HeaderValue::try_from(origin) {
            Ok(value) => self.origin = Some(value),
            Err(e) => self.http_err = Some(e.into()),
        }
        self
    }

    /// Set max frame size
    ///
    /// By default max size is set to 64kb
    pub fn max_frame_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    /// Set request header
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
        where HeaderName: HttpTryFrom<K>, HeaderValue: HttpTryFrom<V>
    {
        self.request.header(key, value);
        self
    }

    /// Connect to websocket server and do ws handshake
    pub fn connect(&mut self) -> WsClientHandshake {
        if let Some(e) = self.err.take() {
            WsClientHandshake::error(e)
        }
        else if let Some(e) = self.http_err.take() {
            WsClientHandshake::error(e.into())
        } else {
            // origin
            if let Some(origin) = self.origin.take() {
                self.request.set_header(header::ORIGIN, origin);
            }

            self.request.upgrade();
            self.request.set_header(header::UPGRADE, "websocket");
            self.request.set_header(header::CONNECTION, "upgrade");
            self.request.set_header("SEC-WEBSOCKET-VERSION", "13");

            if let Some(protocols) = self.protocols.take() {
                self.request.set_header("SEC-WEBSOCKET-PROTOCOL", protocols.as_str());
            }
            let request = match self.request.finish() {
                Ok(req) => req,
                Err(err) => return WsClientHandshake::error(err.into()),
            };

            if request.uri().host().is_none() {
                return WsClientHandshake::error(WsClientError::InvalidUrl)
            }
            if let Some(scheme) = request.uri().scheme_part() {
                if scheme != "http" && scheme != "https" && scheme != "ws" && scheme != "wss" {
                    return WsClientHandshake::error(WsClientError::InvalidUrl)
                }
            } else {
                return WsClientHandshake::error(WsClientError::InvalidUrl)
            }

            // start handshake
            WsClientHandshake::new(request, &self.conn, self.max_size)
        }
    }
}

struct WsInner {
    tx: UnboundedSender<Bytes>,
    rx: PayloadHelper<ClientResponse>,
    closed: bool,
}

pub struct WsClientHandshake {
    request: Option<SendRequest>,
    tx: Option<UnboundedSender<Bytes>>,
    key: String,
    error: Option<WsClientError>,
    max_size: usize,
}

impl WsClientHandshake {
    fn new(mut request: ClientRequest,
           conn: &Addr<Unsync, ClientConnector>, max_size: usize) -> WsClientHandshake
    {
        // Generate a random key for the `Sec-WebSocket-Key` header.
        // a base64-encoded (see Section 4 of [RFC4648]) value that,
        // when decoded, is 16 bytes in length (RFC 6455)
        let sec_key: [u8; 16] = rand::random();
        let key = base64::encode(&sec_key);

        request.headers_mut().insert(
            HeaderName::try_from("SEC-WEBSOCKET-KEY").unwrap(),
            HeaderValue::try_from(key.as_str()).unwrap());

        let (tx, rx) = unbounded();
        request.set_body(Body::Streaming(
            Box::new(rx.map_err(|_| io::Error::new(
                io::ErrorKind::Other, "disconnected").into()))));

        WsClientHandshake {
            key,
            max_size,
            request: Some(request.with_connector(conn.clone())),
            tx: Some(tx),
            error: None,
        }
    }

    fn error(err: WsClientError) -> WsClientHandshake {
        WsClientHandshake {
            key: String::new(),
            request: None,
            tx: None,
            error: Some(err),
            max_size: 0
        }
    }
}

impl Future for WsClientHandshake {
    type Item = (WsClientReader, WsClientWriter);
    type Error = WsClientError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(err) = self.error.take() {
            return Err(err)
        }

        let resp = match self.request.as_mut().unwrap().poll()? {
            Async::Ready(response) => {
                self.request.take();
                response
            },
            Async::NotReady => return Ok(Async::NotReady)
        };

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

        let inner = WsInner {
            tx: self.tx.take().unwrap(),
            rx: PayloadHelper::new(resp),
            closed: false,
        };

        let inner = Rc::new(UnsafeCell::new(inner));
        Ok(Async::Ready(
            (WsClientReader{inner: Rc::clone(&inner), max_size: self.max_size},
             WsClientWriter{inner})))
    }
}


pub struct WsClientReader {
    inner: Rc<UnsafeCell<WsInner>>,
    max_size: usize,
}

impl fmt::Debug for WsClientReader {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "WsClientReader()")
    }
}

impl WsClientReader {
    #[inline]
    fn as_mut(&mut self) -> &mut WsInner {
        unsafe{ &mut *self.inner.get() }
    }
}

impl Stream for WsClientReader {
    type Item = Message;
    type Error = WsError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let max_size = self.max_size;
        let inner = self.as_mut();
        if inner.closed {
            return Ok(Async::Ready(None))
        }

        // read
        match Frame::parse(&mut inner.rx, false, max_size) {
            Ok(Async::Ready(Some(frame))) => {
                let (finished, opcode, payload) = frame.unpack();

                // continuation is not supported
                if !finished {
                    inner.closed = true;
                    return Err(WsError::NoContinuation)
                }

                match opcode {
                    OpCode::Continue => unimplemented!(),
                    OpCode::Bad => {
                        inner.closed = true;
                        Err(WsError::BadOpCode)
                    },
                    OpCode::Close => {
                        inner.closed = true;
                        let code = NetworkEndian::read_uint(payload.as_ref(), 2) as u16;
                        Ok(Async::Ready(Some(Message::Close(CloseCode::from(code)))))
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
                            Err(_) => {
                                inner.closed = true;
                                Err(WsError::BadEncoding)
                            }
                        }
                    }
                }
            }
            Ok(Async::Ready(None)) => Ok(Async::Ready(None)),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => {
                inner.closed = true;
                Err(e)
            }
        }
    }
}

pub struct WsClientWriter {
    inner: Rc<UnsafeCell<WsInner>>
}

impl WsClientWriter {
    #[inline]
    fn as_mut(&mut self) -> &mut WsInner {
        unsafe{ &mut *self.inner.get() }
    }
}

impl WsClientWriter {

    /// Write payload
    #[inline]
    fn write(&mut self, mut data: Binary) {
        if !self.as_mut().closed {
            let _ = self.as_mut().tx.unbounded_send(data.take());
        } else {
            warn!("Trying to write to disconnected response");
        }
    }

    /// Send text frame
    #[inline]
    pub fn text<T: Into<String>>(&mut self, text: T) {
        self.write(Frame::message(text.into(), OpCode::Text, true, true));
    }

    /// Send binary frame
    #[inline]
    pub fn binary<B: Into<Binary>>(&mut self, data: B) {
        self.write(Frame::message(data, OpCode::Binary, true, true));
    }

    /// Send ping frame
    #[inline]
    pub fn ping(&mut self, message: &str) {
        self.write(Frame::message(Vec::from(message), OpCode::Ping, true, true));
    }

    /// Send pong frame
    #[inline]
    pub fn pong(&mut self, message: &str) {
        self.write(Frame::message(Vec::from(message), OpCode::Pong, true, true));
    }

    /// Send close frame
    #[inline]
    pub fn close(&mut self, code: CloseCode, reason: &str) {
        self.write(Frame::close(code, reason, true));
    }
}
