//! `WebSocket` support for Actix
//!
//! To setup a `WebSocket`, first do web socket handshake then on success convert `Payload`
//! into a `WsStream` stream and then use `WsWriter` to communicate with the peer.
//!
//! ## Example
//!
//! ```rust
//! # extern crate actix;
//! # extern crate actix_web;
//! # use actix::*;
//! # use actix_web::*;
//! use actix_web::ws;
//!
//! // do websocket handshake and start actor
//! fn ws_index(req: HttpRequest) -> Result<HttpResponse> {
//!     ws::start(req, Ws)
//! }
//!
//! struct Ws;
//!
//! impl Actor for Ws {
//!     type Context = ws::WebsocketContext<Self>;
//! }
//!
//! // Handler for ws::Message messages
//! impl StreamHandler<ws::Message, ws::ProtocolError> for Ws {
//!
//!     fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context) {
//!         match msg {
//!             ws::Message::Ping(msg) => ctx.pong(&msg),
//!             ws::Message::Text(text) => ctx.text(text),
//!             ws::Message::Binary(bin) => ctx.binary(bin),
//!             _ => (),
//!         }
//!     }
//! }
//! #
//! # fn main() {
//! #    App::new()
//! #      .resource("/ws/", |r| r.f(ws_index))  // <- register websocket route
//! #      .finish();
//! # }
//! ```
use bytes::Bytes;
use http::{Method, StatusCode, header};
use futures::{Async, Poll, Stream};
use byteorder::{ByteOrder, NetworkEndian};

use actix::{Actor, AsyncContext, StreamHandler};

use body::Binary;
use payload::PayloadHelper;
use error::{Error, PayloadError, ResponseError};
use httpmessage::HttpMessage;
use httprequest::HttpRequest;
use httpresponse::{ConnectionType, HttpResponse, HttpResponseBuilder};

mod frame;
mod proto;
mod context;
mod mask;
mod client;

pub use self::frame::Frame;
pub use self::proto::OpCode;
pub use self::proto::CloseCode;
pub use self::context::WebsocketContext;
pub use self::client::{Client, ClientError,
                       ClientReader, ClientWriter, ClientHandshake};

/// Websocket protocol errors
#[derive(Fail, Debug)]
pub enum ProtocolError {
    /// Received an unmasked frame from client
    #[fail(display="Received an unmasked frame from client")]
    UnmaskedFrame,
    /// Received a masked frame from server
    #[fail(display="Received a masked frame from server")]
    MaskedFrame,
    /// Encountered invalid opcode
    #[fail(display="Invalid opcode: {}", _0)]
    InvalidOpcode(u8),
    /// Invalid control frame length
    #[fail(display="Invalid control frame length: {}", _0)]
    InvalidLength(usize),
    /// Bad web socket op code
    #[fail(display="Bad web socket op code")]
    BadOpCode,
    /// A payload reached size limit.
    #[fail(display="A payload reached size limit.")]
    Overflow,
    /// Continuation is not supported
    #[fail(display="Continuation is not supported.")]
    NoContinuation,
    /// Bad utf-8 encoding
    #[fail(display="Bad utf-8 encoding.")]
    BadEncoding,
    /// Payload error
    #[fail(display="Payload error: {}", _0)]
    Payload(#[cause] PayloadError),
}

impl ResponseError for ProtocolError {}

impl From<PayloadError> for ProtocolError {
    fn from(err: PayloadError) -> ProtocolError {
        ProtocolError::Payload(err)
    }
}

/// Websocket handshake errors
#[derive(Fail, PartialEq, Debug)]
pub enum HandshakeError {
    /// Only get method is allowed
    #[fail(display="Method not allowed")]
    GetMethodRequired,
    /// Upgrade header if not set to websocket
    #[fail(display="Websocket upgrade is expected")]
    NoWebsocketUpgrade,
    /// Connection header is not set to upgrade
    #[fail(display="Connection upgrade is expected")]
    NoConnectionUpgrade,
    /// Websocket version header is not set
    #[fail(display="Websocket version header is required")]
    NoVersionHeader,
    /// Unsupported websocket version
    #[fail(display="Unsupported version")]
    UnsupportedVersion,
    /// Websocket key is not set or wrong
    #[fail(display="Unknown websocket key")]
    BadWebsocketKey,
}

impl ResponseError for HandshakeError {

    fn error_response(&self) -> HttpResponse {
        match *self {
            HandshakeError::GetMethodRequired => {
                HttpResponse::MethodNotAllowed().header(header::ALLOW, "GET").finish()
            }
            HandshakeError::NoWebsocketUpgrade => HttpResponse::BadRequest()
                .reason("No WebSocket UPGRADE header found").finish(),
            HandshakeError::NoConnectionUpgrade => HttpResponse::BadRequest()
                .reason("No CONNECTION upgrade").finish(),
            HandshakeError::NoVersionHeader => HttpResponse::BadRequest()
                .reason("Websocket version header is required").finish(),
            HandshakeError::UnsupportedVersion => HttpResponse::BadRequest()
                .reason("Unsupported version").finish(),
            HandshakeError::BadWebsocketKey => HttpResponse::BadRequest()
                .reason("Handshake error").finish(),
        }
    }
}

/// `WebSocket` Message
#[derive(Debug, PartialEq, Message)]
pub enum Message {
    Text(String),
    Binary(Binary),
    Ping(String),
    Pong(String),
    Close(CloseCode),
}

/// Do websocket handshake and start actor
pub fn start<A, S>(req: HttpRequest<S>, actor: A) -> Result<HttpResponse, Error>
    where A: Actor<Context=WebsocketContext<A, S>> + StreamHandler<Message, ProtocolError>,
          S: 'static
{
    let mut resp = handshake(&req)?;
    let stream = WsStream::new(req.clone());

    let mut ctx = WebsocketContext::new(req, actor);
    ctx.add_stream(stream);

    Ok(resp.body(ctx))
}

/// Prepare `WebSocket` handshake response.
///
/// This function returns handshake `HttpResponse`, ready to send to peer.
/// It does not perform any IO.
///
// /// `protocols` is a sequence of known protocols. On successful handshake,
// /// the returned response headers contain the first protocol in this list
// /// which the server also knows.
pub fn handshake<S>(req: &HttpRequest<S>) -> Result<HttpResponseBuilder, HandshakeError> {
    // WebSocket accepts only GET
    if *req.method() != Method::GET {
        return Err(HandshakeError::GetMethodRequired)
    }

    // Check for "UPGRADE" to websocket header
    let has_hdr = if let Some(hdr) = req.headers().get(header::UPGRADE) {
        if let Ok(s) = hdr.to_str() {
            s.to_lowercase().contains("websocket")
        } else {
            false
        }
    } else {
        false
    };
    if !has_hdr {
        return Err(HandshakeError::NoWebsocketUpgrade)
    }

    // Upgrade connection
    if !req.upgrade() {
        return Err(HandshakeError::NoConnectionUpgrade)
    }

    // check supported version
    if !req.headers().contains_key(header::SEC_WEBSOCKET_VERSION) {
        return Err(HandshakeError::NoVersionHeader)
    }
    let supported_ver = {
        if let Some(hdr) = req.headers().get(header::SEC_WEBSOCKET_VERSION) {
            hdr == "13" || hdr == "8" || hdr == "7"
        } else {
            false
        }
    };
    if !supported_ver {
        return Err(HandshakeError::UnsupportedVersion)
    }

    // check client handshake for validity
    if !req.headers().contains_key(header::SEC_WEBSOCKET_KEY) {
        return Err(HandshakeError::BadWebsocketKey)
    }
    let key = {
        let key = req.headers().get(header::SEC_WEBSOCKET_KEY).unwrap();
        proto::hash_key(key.as_ref())
    };

    Ok(HttpResponse::build(StatusCode::SWITCHING_PROTOCOLS)
       .connection_type(ConnectionType::Upgrade)
       .header(header::UPGRADE, "websocket")
       .header(header::TRANSFER_ENCODING, "chunked")
       .header(header::SEC_WEBSOCKET_ACCEPT, key.as_str())
       .take())
}

/// Maps `Payload` stream into stream of `ws::Message` items
pub struct WsStream<S> {
    rx: PayloadHelper<S>,
    closed: bool,
    max_size: usize,
}

impl<S> WsStream<S> where S: Stream<Item=Bytes, Error=PayloadError> {
    /// Create new websocket frames stream
    pub fn new(stream: S) -> WsStream<S> {
        WsStream { rx: PayloadHelper::new(stream),
                   closed: false,
                   max_size: 65_536,
        }
    }

    /// Set max frame size
    ///
    /// By default max size is set to 64kb
    pub fn max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }
}

impl<S> Stream for WsStream<S> where S: Stream<Item=Bytes, Error=PayloadError> {
    type Item = Message;
    type Error = ProtocolError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if self.closed {
            return Ok(Async::Ready(None))
        }

        match Frame::parse(&mut self.rx, true, self.max_size) {
            Ok(Async::Ready(Some(frame))) => {
                let (finished, opcode, payload) = frame.unpack();

                // continuation is not supported
                if !finished {
                    self.closed = true;
                    return Err(ProtocolError::NoContinuation)
                }

                match opcode {
                    OpCode::Continue => Err(ProtocolError::NoContinuation),
                    OpCode::Bad => {
                        self.closed = true;
                        Err(ProtocolError::BadOpCode)
                    }
                    OpCode::Close => {
                        self.closed = true;
                        let code = NetworkEndian::read_uint(payload.as_ref(), 2) as u16;
                        Ok(Async::Ready(
                            Some(Message::Close(CloseCode::from(code)))))
                    },
                    OpCode::Ping =>
                        Ok(Async::Ready(Some(
                            Message::Ping(
                                String::from_utf8_lossy(payload.as_ref()).into())))),
                    OpCode::Pong =>
                        Ok(Async::Ready(Some(
                            Message::Pong(String::from_utf8_lossy(payload.as_ref()).into())))),
                    OpCode::Binary =>
                        Ok(Async::Ready(Some(Message::Binary(payload)))),
                    OpCode::Text => {
                        let tmp = Vec::from(payload.as_ref());
                        match String::from_utf8(tmp) {
                            Ok(s) =>
                                Ok(Async::Ready(Some(Message::Text(s)))),
                            Err(_) => {
                                self.closed = true;
                                Err(ProtocolError::BadEncoding)
                            }
                        }
                    }
                }
            }
            Ok(Async::Ready(None)) => Ok(Async::Ready(None)),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => {
                self.closed = true;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use http::{Method, HeaderMap, Version, Uri, header};

    #[test]
    fn test_handshake() {
        let req = HttpRequest::new(Method::POST, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, HeaderMap::new(), None);
        assert_eq!(HandshakeError::GetMethodRequired, handshake(&req).err().unwrap());

        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, HeaderMap::new(), None);
        assert_eq!(HandshakeError::NoWebsocketUpgrade, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("test"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(HandshakeError::NoWebsocketUpgrade, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(HandshakeError::NoConnectionUpgrade, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(HandshakeError::NoVersionHeader, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        headers.insert(header::SEC_WEBSOCKET_VERSION,
                       header::HeaderValue::from_static("5"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(HandshakeError::UnsupportedVersion, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        headers.insert(header::SEC_WEBSOCKET_VERSION,
                       header::HeaderValue::from_static("13"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(HandshakeError::BadWebsocketKey, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        headers.insert(header::SEC_WEBSOCKET_VERSION,
                       header::HeaderValue::from_static("13"));
        headers.insert(header::SEC_WEBSOCKET_KEY,
                       header::HeaderValue::from_static("13"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(StatusCode::SWITCHING_PROTOCOLS,
                   handshake(&req).unwrap().finish().status());
    }

    #[test]
    fn test_wserror_http_response() {
        let resp: HttpResponse = HandshakeError::GetMethodRequired.error_response();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        let resp: HttpResponse = HandshakeError::NoWebsocketUpgrade.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: HttpResponse = HandshakeError::NoConnectionUpgrade.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: HttpResponse = HandshakeError::NoVersionHeader.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: HttpResponse = HandshakeError::UnsupportedVersion.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp: HttpResponse = HandshakeError::BadWebsocketKey.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
