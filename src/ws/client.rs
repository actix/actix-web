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

use super::{Message, ProtocolError};
use super::frame::Frame;
use super::proto::{CloseCode, OpCode};


/// Backward compatibility
#[doc(hidden)]
#[deprecated(since="0.4.2", note="please use `ws::Client` instead")]
pub type WsClient = Client;
#[doc(hidden)]
#[deprecated(since="0.4.2", note="please use `ws::ClientError` instead")]
pub type WsClientError = ClientError;
#[doc(hidden)]
#[deprecated(since="0.4.2", note="please use `ws::ClientReader` instead")]
pub type WsClientReader = ClientReader;
#[doc(hidden)]
#[deprecated(since="0.4.2", note="please use `ws::ClientWriter` instead")]
pub type WsClientWriter = ClientWriter;
#[doc(hidden)]
#[deprecated(since="0.4.2", note="please use `ws::ClientHandshake` instead")]
pub type WsClientHandshake = ClientHandshake;


/// Websocket client error
#[derive(Fail, Debug)]
pub enum ClientError {
    #[fail(display="Invalid url")]
    InvalidUrl,
    #[fail(display="Invalid response status")]
    InvalidResponseStatus(StatusCode),
    #[fail(display="Invalid upgrade header")]
    InvalidUpgradeHeader,
    #[fail(display="Invalid connection header")]
    InvalidConnectionHeader(HeaderValue),
    #[fail(display="Missing CONNECTION header")]
    MissingConnectionHeader,
    #[fail(display="Missing SEC-WEBSOCKET-ACCEPT header")]
    MissingWebSocketAcceptHeader,
    #[fail(display="Invalid challenge response")]
    InvalidChallengeResponse(String, HeaderValue),
    #[fail(display="Http parsing error")]
    Http(HttpError),
    #[fail(display="Url parsing error")]
    Url(UrlParseError),
    #[fail(display="Response parsing error")]
    ResponseParseError(HttpResponseParserError),
    #[fail(display="{}", _0)]
    SendRequest(SendRequestError),
    #[fail(display="{}", _0)]
    Protocol(#[cause] ProtocolError),
    #[fail(display="{}", _0)]
    Io(io::Error),
    #[fail(display="Disconnected")]
    Disconnected,
}

impl From<HttpError> for ClientError {
    fn from(err: HttpError) -> ClientError {
        ClientError::Http(err)
    }
}

impl From<UrlParseError> for ClientError {
    fn from(err: UrlParseError) -> ClientError {
        ClientError::Url(err)
    }
}

impl From<SendRequestError> for ClientError {
    fn from(err: SendRequestError) -> ClientError {
        ClientError::SendRequest(err)
    }
}

impl From<ProtocolError> for ClientError {
    fn from(err: ProtocolError) -> ClientError {
        ClientError::Protocol(err)
    }
}

impl From<io::Error> for ClientError {
    fn from(err: io::Error) -> ClientError {
        ClientError::Io(err)
    }
}

impl From<HttpResponseParserError> for ClientError {
    fn from(err: HttpResponseParserError) -> ClientError {
        ClientError::ResponseParseError(err)
    }
}

/// `WebSocket` client
///
/// Example of `WebSocket` client usage is available in
/// [websocket example](
/// https://github.com/actix/actix-web/blob/master/examples/websocket/src/client.rs#L24)
pub struct Client {
    request: ClientRequestBuilder,
    err: Option<ClientError>,
    http_err: Option<HttpError>,
    origin: Option<HeaderValue>,
    protocols: Option<String>,
    conn: Addr<Unsync, ClientConnector>,
    max_size: usize,
}

impl Client {

    /// Create new websocket connection
    pub fn new<S: AsRef<str>>(uri: S) -> Client {
        Client::with_connector(uri, ClientConnector::from_registry())
    }

    /// Create new websocket connection with custom `ClientConnector`
    pub fn with_connector<S: AsRef<str>>(uri: S, conn: Addr<Unsync, ClientConnector>) -> Client {
        let mut cl = Client {
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
    pub fn connect(&mut self) -> ClientHandshake {
        if let Some(e) = self.err.take() {
            ClientHandshake::error(e)
        }
        else if let Some(e) = self.http_err.take() {
            ClientHandshake::error(e.into())
        } else {
            // origin
            if let Some(origin) = self.origin.take() {
                self.request.set_header(header::ORIGIN, origin);
            }

            self.request.upgrade();
            self.request.set_header(header::UPGRADE, "websocket");
            self.request.set_header(header::CONNECTION, "upgrade");
            self.request.set_header(header::SEC_WEBSOCKET_VERSION, "13");
            self.request.with_connector(self.conn.clone());

            if let Some(protocols) = self.protocols.take() {
                self.request.set_header(header::SEC_WEBSOCKET_PROTOCOL, protocols.as_str());
            }
            let request = match self.request.finish() {
                Ok(req) => req,
                Err(err) => return ClientHandshake::error(err.into()),
            };

            if request.uri().host().is_none() {
                return ClientHandshake::error(ClientError::InvalidUrl)
            }
            if let Some(scheme) = request.uri().scheme_part() {
                if scheme != "http" && scheme != "https" && scheme != "ws" && scheme != "wss" {
                    return ClientHandshake::error(ClientError::InvalidUrl)
                }
            } else {
                return ClientHandshake::error(ClientError::InvalidUrl)
            }

            // start handshake
            ClientHandshake::new(request, self.max_size)
        }
    }
}

struct Inner {
    tx: UnboundedSender<Bytes>,
    rx: PayloadHelper<ClientResponse>,
    closed: bool,
}

pub struct ClientHandshake {
    request: Option<SendRequest>,
    tx: Option<UnboundedSender<Bytes>>,
    key: String,
    error: Option<ClientError>,
    max_size: usize,
}

impl ClientHandshake {
    fn new(mut request: ClientRequest, max_size: usize) -> ClientHandshake
    {
        // Generate a random key for the `Sec-WebSocket-Key` header.
        // a base64-encoded (see Section 4 of [RFC4648]) value that,
        // when decoded, is 16 bytes in length (RFC 6455)
        let sec_key: [u8; 16] = rand::random();
        let key = base64::encode(&sec_key);

        request.headers_mut().insert(
            header::SEC_WEBSOCKET_KEY,
            HeaderValue::try_from(key.as_str()).unwrap());

        let (tx, rx) = unbounded();
        request.set_body(Body::Streaming(
            Box::new(rx.map_err(|_| io::Error::new(
                io::ErrorKind::Other, "disconnected").into()))));

        ClientHandshake {
            key,
            max_size,
            request: Some(request.send()),
            tx: Some(tx),
            error: None,
        }
    }

    fn error(err: ClientError) -> ClientHandshake {
        ClientHandshake {
            key: String::new(),
            request: None,
            tx: None,
            error: Some(err),
            max_size: 0
        }
    }
}

impl Future for ClientHandshake {
    type Item = (ClientReader, ClientWriter);
    type Error = ClientError;

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
            return Err(ClientError::InvalidResponseStatus(resp.status()))
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
            trace!("Invalid upgrade header");
            return Err(ClientError::InvalidUpgradeHeader)
        }
        // Check for "CONNECTION" header
        if let Some(conn) = resp.headers().get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                if !s.to_lowercase().contains("upgrade") {
                    trace!("Invalid connection header: {}", s);
                    return Err(ClientError::InvalidConnectionHeader(conn.clone()))
                }
            } else {
                trace!("Invalid connection header: {:?}", conn);
                return Err(ClientError::InvalidConnectionHeader(conn.clone()))
            }
        } else {
            trace!("Missing connection header");
            return Err(ClientError::MissingConnectionHeader)
        }

        if let Some(key) = resp.headers().get(header::SEC_WEBSOCKET_ACCEPT)
        {
            // field is constructed by concatenating /key/
            // with the string "258EAFA5-E914-47DA-95CA-C5AB0DC85B11" (RFC 6455)
            const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
            let mut sha1 = Sha1::new();
            sha1.update(self.key.as_ref());
            sha1.update(WS_GUID);
            let encoded = base64::encode(&sha1.digest().bytes());
            if key.as_bytes() != encoded.as_bytes() {
                trace!(
                    "Invalid challenge response: expected: {} received: {:?}",
                    encoded, key);
                return Err(ClientError::InvalidChallengeResponse(encoded, key.clone()));
            }
        } else {
            trace!("Missing SEC-WEBSOCKET-ACCEPT header");
            return Err(ClientError::MissingWebSocketAcceptHeader)
        };

        let inner = Inner {
            tx: self.tx.take().unwrap(),
            rx: PayloadHelper::new(resp),
            closed: false,
        };

        let inner = Rc::new(UnsafeCell::new(inner));
        Ok(Async::Ready(
            (ClientReader{inner: Rc::clone(&inner), max_size: self.max_size},
             ClientWriter{inner})))
    }
}


pub struct ClientReader {
    inner: Rc<UnsafeCell<Inner>>,
    max_size: usize,
}

impl fmt::Debug for ClientReader {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ws::ClientReader()")
    }
}

impl ClientReader {
    #[inline]
    fn as_mut(&mut self) -> &mut Inner {
        unsafe{ &mut *self.inner.get() }
    }
}

impl Stream for ClientReader {
    type Item = Message;
    type Error = ProtocolError;

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
                    return Err(ProtocolError::NoContinuation)
                }

                match opcode {
                    OpCode::Continue => unimplemented!(),
                    OpCode::Bad => {
                        inner.closed = true;
                        Err(ProtocolError::BadOpCode)
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
                                Err(ProtocolError::BadEncoding)
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

pub struct ClientWriter {
    inner: Rc<UnsafeCell<Inner>>
}

impl ClientWriter {
    #[inline]
    fn as_mut(&mut self) -> &mut Inner {
        unsafe{ &mut *self.inner.get() }
    }
}

impl ClientWriter {

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
