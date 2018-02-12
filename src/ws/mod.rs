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
//! // Define Handler for ws::Message message
//! impl Handler<ws::Message> for Ws {
//!     type Result = ();
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
//! #    Application::new()
//! #      .resource("/ws/", |r| r.f(ws_index))  // <- register websocket route
//! #      .finish();
//! # }
//! ```
use bytes::BytesMut;
use http::{Method, StatusCode, header};
use futures::{Async, Poll, Stream};

use actix::{Actor, AsyncContext, Handler};

use body::Binary;
use payload::ReadAny;
use error::{Error, WsHandshakeError};
use httprequest::HttpRequest;
use httpresponse::{ConnectionType, HttpResponse, HttpResponseBuilder};

mod frame;
mod proto;
mod context;
mod mask;
mod client;

use self::frame::Frame;
use self::proto::{hash_key, OpCode};
pub use self::proto::CloseCode;
pub use self::context::WebsocketContext;
pub use self::client::{WsClient, WsClientError, WsClientReader, WsClientWriter, WsClientFuture};

const SEC_WEBSOCKET_ACCEPT: &str = "SEC-WEBSOCKET-ACCEPT";
const SEC_WEBSOCKET_KEY: &str = "SEC-WEBSOCKET-KEY";
const SEC_WEBSOCKET_VERSION: &str = "SEC-WEBSOCKET-VERSION";
// const SEC_WEBSOCKET_PROTOCOL: &'static str = "SEC-WEBSOCKET-PROTOCOL";


/// `WebSocket` Message
#[derive(Debug, PartialEq, Message)]
pub enum Message {
    Text(String),
    Binary(Binary),
    Ping(String),
    Pong(String),
    Close,
    Closed,
    Error
}

/// Do websocket handshake and start actor
pub fn start<A, S>(mut req: HttpRequest<S>, actor: A) -> Result<HttpResponse, Error>
    where A: Actor<Context=WebsocketContext<A, S>> + Handler<Message>,
          S: 'static
{
    let mut resp = handshake(&req)?;
    let stream = WsStream::new(req.payload_mut().readany());

    let mut ctx = WebsocketContext::new(req, actor);
    ctx.add_message_stream(stream);

    Ok(resp.body(ctx)?)
}

/// Prepare `WebSocket` handshake response.
///
/// This function returns handshake `HttpResponse`, ready to send to peer.
/// It does not perform any IO.
///
// /// `protocols` is a sequence of known protocols. On successful handshake,
// /// the returned response headers contain the first protocol in this list
// /// which the server also knows.
pub fn handshake<S>(req: &HttpRequest<S>) -> Result<HttpResponseBuilder, WsHandshakeError> {
    // WebSocket accepts only GET
    if *req.method() != Method::GET {
        return Err(WsHandshakeError::GetMethodRequired)
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
        return Err(WsHandshakeError::NoWebsocketUpgrade)
    }

    // Upgrade connection
    if !req.upgrade() {
        return Err(WsHandshakeError::NoConnectionUpgrade)
    }

    // check supported version
    if !req.headers().contains_key(SEC_WEBSOCKET_VERSION) {
        return Err(WsHandshakeError::NoVersionHeader)
    }
    let supported_ver = {
        if let Some(hdr) = req.headers().get(SEC_WEBSOCKET_VERSION) {
            hdr == "13" || hdr == "8" || hdr == "7"
        } else {
            false
        }
    };
    if !supported_ver {
        return Err(WsHandshakeError::UnsupportedVersion)
    }

    // check client handshake for validity
    if !req.headers().contains_key(SEC_WEBSOCKET_KEY) {
        return Err(WsHandshakeError::BadWebsocketKey)
    }
    let key = {
        let key = req.headers().get(SEC_WEBSOCKET_KEY).unwrap();
        hash_key(key.as_ref())
    };

    Ok(HttpResponse::build(StatusCode::SWITCHING_PROTOCOLS)
       .connection_type(ConnectionType::Upgrade)
       .header(header::UPGRADE, "websocket")
       .header(header::TRANSFER_ENCODING, "chunked")
       .header(SEC_WEBSOCKET_ACCEPT, key.as_str())
       .take())
}

/// Maps `Payload` stream into stream of `ws::Message` items
pub struct WsStream {
    rx: ReadAny,
    buf: BytesMut,
    closed: bool,
    error_sent: bool,
}

impl WsStream {
    pub fn new(payload: ReadAny) -> WsStream {
        WsStream { rx: payload,
                   buf: BytesMut::new(),
                   closed: false,
                   error_sent: false }
    }
}

impl Stream for WsStream {
    type Item = Message;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let mut done = false;

        if !self.closed {
            loop {
                match self.rx.poll() {
                    Ok(Async::Ready(Some(chunk))) => {
                        self.buf.extend_from_slice(&chunk)
                    }
                    Ok(Async::Ready(None)) => {
                        done = true;
                        self.closed = true;
                        break;
                    }
                    Ok(Async::NotReady) => break,
                    Err(_) => {
                        self.closed = true;
                        break;
                    }
                }
            }
        }

        loop {
            match Frame::parse(&mut self.buf, true) {
                Ok(Some(frame)) => {
                    // trace!("WsFrame {}", frame);
                    let (_finished, opcode, payload) = frame.unpack();

                    match opcode {
                        OpCode::Continue => continue,
                        OpCode::Bad =>
                            return Ok(Async::Ready(Some(Message::Error))),
                        OpCode::Close => {
                            self.closed = true;
                            self.error_sent = true;
                            return Ok(Async::Ready(Some(Message::Closed)))
                        },
                        OpCode::Ping =>
                            return Ok(Async::Ready(Some(
                                Message::Ping(
                                    String::from_utf8_lossy(payload.as_ref()).into())))),
                        OpCode::Pong =>
                            return Ok(Async::Ready(Some(
                                Message::Pong(
                                    String::from_utf8_lossy(payload.as_ref()).into())))),
                        OpCode::Binary =>
                            return Ok(Async::Ready(Some(Message::Binary(payload)))),
                        OpCode::Text => {
                            let tmp = Vec::from(payload.as_ref());
                            match String::from_utf8(tmp) {
                                Ok(s) =>
                                    return Ok(Async::Ready(Some(Message::Text(s)))),
                                Err(_) =>
                                    return Ok(Async::Ready(Some(Message::Error))),
                            }
                        }
                    }
                }
                Ok(None) => {
                    if done {
                        return Ok(Async::Ready(None))
                    } else if self.closed {
                        if !self.error_sent {
                            self.error_sent = true;
                            return Ok(Async::Ready(Some(Message::Closed)))
                        } else {
                            return Ok(Async::Ready(None))
                        }
                    } else {
                        return Ok(Async::NotReady)
                    }
                },
                Err(_) => {
                    self.closed = true;
                    self.error_sent = true;
                    return Ok(Async::Ready(Some(Message::Error)));
                }
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
        assert_eq!(WsHandshakeError::GetMethodRequired, handshake(&req).err().unwrap());

        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, HeaderMap::new(), None);
        assert_eq!(WsHandshakeError::NoWebsocketUpgrade, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("test"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(WsHandshakeError::NoWebsocketUpgrade, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(WsHandshakeError::NoConnectionUpgrade, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(WsHandshakeError::NoVersionHeader, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        headers.insert(SEC_WEBSOCKET_VERSION,
                       header::HeaderValue::from_static("5"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(WsHandshakeError::UnsupportedVersion, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        headers.insert(SEC_WEBSOCKET_VERSION,
                       header::HeaderValue::from_static("13"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(WsHandshakeError::BadWebsocketKey, handshake(&req).err().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        headers.insert(SEC_WEBSOCKET_VERSION,
                       header::HeaderValue::from_static("13"));
        headers.insert(SEC_WEBSOCKET_KEY,
                       header::HeaderValue::from_static("13"));
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        assert_eq!(StatusCode::SWITCHING_PROTOCOLS,
                   handshake(&req).unwrap().finish().unwrap().status());
    }
}
