//! `WebSocket` support for Actix
//!
//! To setup a `WebSocket`, first do web socket handshake then on success convert `Payload`
//! into a `WsStream` stream and then use `WsWriter` to communicate with the peer.
//!
//! ## Example
//!
//! ```rust
//! extern crate actix;
//! extern crate actix_web;
//!
//! use actix::*;
//! use actix_web::*;
//!
//! // WebSocket Route
//! struct WsRoute;
//!
//! impl Actor for WsRoute {
//!     type Context = HttpContext<Self>;
//! }
//!
//! impl Route for WsRoute {
//!     type State = ();
//!
//!     fn request(req: &mut HttpRequest,
//!                payload: Payload, ctx: &mut HttpContext<Self>) -> RouteResult<Self>
//!     {
//!         // WebSocket handshake
//!         match ws::handshake(&req) {
//!             Ok(resp) => {
//!                 // Send handshake response to peer
//!                 ctx.start(resp);
//!                 // Map Payload into WsStream
//!                 ctx.add_stream(ws::WsStream::new(payload));
//!                 // Start ws messages processing
//!                 Reply::async(WsRoute)
//!             },
//!             Err(err) =>
//!                 Reply::reply(err)
//!         }
//!     }
//! }
//!
//! // Define Handler for ws::Message message
//! impl StreamHandler<ws::Message> for WsRoute {}
//!
//! impl Handler<ws::Message> for WsRoute {
//!     fn handle(&mut self, msg: ws::Message, ctx: &mut HttpContext<Self>)
//!               -> Response<Self, ws::Message>
//!     {
//!         match msg {
//!             ws::Message::Ping(msg) => ws::WsWriter::pong(ctx, &msg),
//!             ws::Message::Text(text) => ws::WsWriter::text(ctx, &text),
//!             ws::Message::Binary(bin) => ws::WsWriter::binary(ctx, bin),
//!             _ => (),
//!         }
//!         Self::empty()
//!     }
//! }
//!
//! fn main() {}
//! ```
use std::vec::Vec;
use http::{Method, StatusCode, header};
use bytes::BytesMut;
use futures::{Async, Poll, Stream};

use actix::{Actor, ResponseType};

use body::Body;
use context::HttpContext;
use route::Route;
use payload::Payload;
use httpcodes::{HTTPBadRequest, HTTPMethodNotAllowed};
use httprequest::HttpRequest;
use httpresponse::{ConnectionType, HttpResponse};

use wsframe;
use wsproto::*;
pub use wsproto::CloseCode;

#[doc(hidden)]
const SEC_WEBSOCKET_ACCEPT: &'static str = "SEC-WEBSOCKET-ACCEPT";
#[doc(hidden)]
const SEC_WEBSOCKET_KEY: &'static str = "SEC-WEBSOCKET-KEY";
#[doc(hidden)]
const SEC_WEBSOCKET_VERSION: &'static str = "SEC-WEBSOCKET-VERSION";
// #[doc(hidden)]
// const SEC_WEBSOCKET_PROTOCOL: &'static str = "SEC-WEBSOCKET-PROTOCOL";


/// `WebSocket` Message
#[derive(Debug)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
    Ping(String),
    Pong(String),
    Close,
    Closed,
    Error
}

impl ResponseType for Message {
    type Item = ();
    type Error = ();
}

/// Prepare `WebSocket` handshake response.
///
/// This function returns handshake `HttpResponse`, ready to send to peer.
/// It does not perform any IO.
///
// /// `protocols` is a sequence of known protocols. On successful handshake,
// /// the returned response headers contain the first protocol in this list
// /// which the server also knows.
pub fn handshake(req: &HttpRequest) -> Result<HttpResponse, HttpResponse> {
    // WebSocket accepts only GET
    if *req.method() != Method::GET {
        return Err(
            HTTPMethodNotAllowed
                .builder()
                .header(header::ALLOW, "GET")
                .finish()?)
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
        return Err(HTTPBadRequest.with_reason("No WebSocket UPGRADE header found"))
    }

    // Upgrade connection
    if !req.upgrade() {
        return Err(HTTPBadRequest.with_reason("No CONNECTION upgrade"))
    }

    // check supported version
    if !req.headers().contains_key(SEC_WEBSOCKET_VERSION) {
        return Err(HTTPBadRequest.with_reason("No websocket version header is required"))
    }
    let supported_ver = {
        if let Some(hdr) = req.headers().get(SEC_WEBSOCKET_VERSION) {
            hdr == "13" || hdr == "8" || hdr == "7"
        } else {
            false
        }
    };
    if !supported_ver {
        return Err(HTTPBadRequest.with_reason("Unsupported version"))
    }

    // check client handshake for validity
    if !req.headers().contains_key(SEC_WEBSOCKET_KEY) {
        return Err(HTTPBadRequest.with_reason("Handshake error"));
    }
    let key = {
        let key = req.headers().get(SEC_WEBSOCKET_KEY).unwrap();
        hash_key(key.as_ref())
    };

    Ok(HttpResponse::builder(StatusCode::SWITCHING_PROTOCOLS)
       .connection_type(ConnectionType::Upgrade)
       .header(header::UPGRADE, "websocket")
       .header(header::TRANSFER_ENCODING, "chunked")
       .header(SEC_WEBSOCKET_ACCEPT, key.as_str())
       .body(Body::Upgrade)?
    )
}


/// Maps `Payload` stream into stream of `ws::Message` items
pub struct WsStream {
    rx: Payload,
    buf: BytesMut,
    closed: bool,
    error_sent: bool,
}

impl WsStream {
    pub fn new(rx: Payload) -> WsStream {
        WsStream { rx: rx, buf: BytesMut::new(), closed: false, error_sent: false }
    }
}

impl Stream for WsStream {
    type Item = Message;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let mut done = false;

        if !self.closed {
            loop {
                match self.rx.readany() {
                    Ok(Async::Ready(Some(chunk))) => {
                        self.buf.extend(chunk.0)
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
            match wsframe::Frame::parse(&mut self.buf) {
                Ok(Some(frame)) => {
                    trace!("WsFrame {}", frame);
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
                                Message::Ping(String::from_utf8_lossy(&payload).into())))),
                        OpCode::Pong =>
                            return Ok(Async::Ready(Some(
                                Message::Pong(String::from_utf8_lossy(&payload).into())))),
                        OpCode::Binary =>
                            return Ok(Async::Ready(Some(Message::Binary(payload)))),
                        OpCode::Text => {
                            match String::from_utf8(payload) {
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


/// `WebSocket` writer
pub struct WsWriter;

impl WsWriter {

    /// Send text frame
    pub fn text<A>(ctx: &mut HttpContext<A>, text: &str)
        where A: Actor<Context=HttpContext<A>> + Route
    {
        let mut frame = wsframe::Frame::message(Vec::from(text), OpCode::Text, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        ctx.write(buf);
    }

    /// Send binary frame
    pub fn binary<A>(ctx: &mut HttpContext<A>, data: Vec<u8>)
        where A: Actor<Context=HttpContext<A>> + Route
    {
        let mut frame = wsframe::Frame::message(data, OpCode::Binary, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        ctx.write(buf);
    }

    /// Send ping frame
    pub fn ping<A>(ctx: &mut HttpContext<A>, message: &str)
        where A: Actor<Context=HttpContext<A>> + Route
    {
        let mut frame = wsframe::Frame::message(Vec::from(message), OpCode::Ping, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        ctx.write(buf);
    }

    /// Send pong frame
    pub fn pong<A>(ctx: &mut HttpContext<A>, message: &str)
        where A: Actor<Context=HttpContext<A>> + Route
    {
        let mut frame = wsframe::Frame::message(Vec::from(message), OpCode::Pong, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        ctx.write(buf);
    }

    /// Send close frame
    pub fn close<A>(ctx: &mut HttpContext<A>, code: CloseCode, reason: &str)
        where A: Actor<Context=HttpContext<A>> + Route
    {
        let mut frame = wsframe::Frame::close(code, reason);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();
        ctx.write(buf);
    }
}

#[cfg(test)]
mod tests {
    use http::{Method, HeaderMap, StatusCode, Version, header};
    use super::{HttpRequest, SEC_WEBSOCKET_VERSION, SEC_WEBSOCKET_KEY, handshake};

    #[test]
    fn test_handshake() {
        let req = HttpRequest::new(Method::POST, "/".to_owned(),
                                   Version::HTTP_11, HeaderMap::new(), String::new());
        match handshake(&req) {
            Err(err) => assert_eq!(err.status(), StatusCode::METHOD_NOT_ALLOWED),
            _ => panic!("should not happen"),
        }

        let req = HttpRequest::new(Method::GET, "/".to_owned(),
                                   Version::HTTP_11, HeaderMap::new(), String::new());
        match handshake(&req) {
            Err(err) => assert_eq!(err.status(), StatusCode::BAD_REQUEST),
            _ => panic!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("test"));
        let req = HttpRequest::new(Method::GET, "/".to_owned(),
                                   Version::HTTP_11, headers, String::new());
        match handshake(&req) {
            Err(err) => assert_eq!(err.status(), StatusCode::BAD_REQUEST),
            _ => panic!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        let req = HttpRequest::new(Method::GET, "/".to_owned(),
                                   Version::HTTP_11, headers, String::new());
        match handshake(&req) {
            Err(err) => assert_eq!(err.status(), StatusCode::BAD_REQUEST),
            _ => panic!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        let req = HttpRequest::new(Method::GET, "/".to_owned(),
                                   Version::HTTP_11, headers, String::new());
        match handshake(&req) {
            Err(err) => assert_eq!(err.status(), StatusCode::BAD_REQUEST),
            _ => panic!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        headers.insert(SEC_WEBSOCKET_VERSION,
                       header::HeaderValue::from_static("5"));
        let req = HttpRequest::new(Method::GET, "/".to_owned(),
                                   Version::HTTP_11, headers, String::new());
        match handshake(&req) {
            Err(err) => assert_eq!(err.status(), StatusCode::BAD_REQUEST),
            _ => panic!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        headers.insert(SEC_WEBSOCKET_VERSION,
                       header::HeaderValue::from_static("13"));
        let req = HttpRequest::new(Method::GET, "/".to_owned(),
                                   Version::HTTP_11, headers, String::new());
        match handshake(&req) {
            Err(err) => assert_eq!(err.status(), StatusCode::BAD_REQUEST),
            _ => panic!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE,
                       header::HeaderValue::from_static("websocket"));
        headers.insert(header::CONNECTION,
                       header::HeaderValue::from_static("upgrade"));
        headers.insert(SEC_WEBSOCKET_VERSION,
                       header::HeaderValue::from_static("13"));
        headers.insert(SEC_WEBSOCKET_KEY,
                       header::HeaderValue::from_static("13"));
        let req = HttpRequest::new(Method::GET, "/".to_owned(),
                                   Version::HTTP_11, headers, String::new());
        match handshake(&req) {
            Ok(resp) => {
                assert_eq!(resp.status(), StatusCode::SWITCHING_PROTOCOLS)
            },
            _ => panic!("should not happen"),
        }
    }
}
