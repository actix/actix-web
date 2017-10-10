//! `WebSocket` support for Actix
//!
//! To setup a `WebSocket`, first do web socket handshake then on success convert `Payload`
//! into a `WsStream` stream and then use `WsWriter` to communicate with the peer.
//!
//! ## Example
//!
//! ```rust
//! extern crate actix;
//! extern crate actix_http;
//! use actix::prelude::*;
//! use actix_http::*;
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
//!     fn request(req: HttpRequest, payload: Payload, ctx: &mut HttpContext<Self>) -> Reply<Self>
//!     {
//!         // WebSocket handshake
//!         match ws::handshake(req) {
//!             Ok(resp) => {
//!                 // Send handshake response to peer
//!                 ctx.start(resp);
//!                 // Map Payload into WsStream
//!                 ctx.add_stream(ws::WsStream::new(payload));
//!                 // Start ws messages processing
//!                 Reply::stream(WsRoute)
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
//!             ws::Message::Ping(msg) => ws::WsWriter::pong(ctx, msg),
//!             ws::Message::Text(text) => ws::WsWriter::text(ctx, text),
//!             ws::Message::Binary(bin) => ws::WsWriter::binary(ctx, bin),
//!             _ => (),
//!         }
//!         Self::empty()
//!     }
//! }
//!
//! impl ResponseType<ws::Message> for WsRoute {
//!     type Item = ();
//!     type Error = ();
//! }
//!
//! fn main() {}
//! ```
use std::vec::Vec;
use std::str::FromStr;
use http::{Method, StatusCode, header};
use bytes::{Bytes, BytesMut};
use futures::{Async, Poll, Stream};

use actix::Actor;

use context::HttpContext;
use route::Route;
use payload::Payload;
use httpcodes::{HTTPBadRequest, HTTPMethodNotAllowed};
use httpmessage::{Body, ConnectionType, HttpRequest, HttpResponse, IntoHttpResponse};

use wsframe;
use wsproto::*;

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

/// Prepare `WebSocket` handshake response.
///
/// This function returns handshake `HttpResponse`, ready to send to peer.
/// It does not perform any IO.
///
// /// `protocols` is a sequence of known protocols. On successful handshake,
// /// the returned response headers contain the first protocol in this list
// /// which the server also knows.
pub fn handshake(req: HttpRequest) -> Result<HttpResponse, HttpResponse> {
    // WebSocket accepts only GET
    if *req.method() != Method::GET {
        return Err(HTTPMethodNotAllowed.response(req))
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
        return Err(HTTPMethodNotAllowed.with_reason(req, "No WebSocket UPGRADE header found"))
    }

    // Upgrade connection
    if !req.is_upgrade() {
        return Err(HTTPBadRequest.with_reason(req, "No CONNECTION upgrade"))
    }

    // check supported version
    if !req.headers().contains_key(SEC_WEBSOCKET_VERSION) {
        return Err(HTTPBadRequest.with_reason(req, "No websocket version header is required"))
    }
    let supported_ver = {
        if let Some(hdr) = req.headers().get(SEC_WEBSOCKET_VERSION) {
            hdr == "13" || hdr == "8" || hdr == "7"
        } else {
            false
        }
    };
    if !supported_ver {
        return Err(HTTPBadRequest.with_reason(req, "Unsupported version"))
    }

    // check client handshake for validity
    if !req.headers().contains_key(SEC_WEBSOCKET_KEY) {
        return Err(HTTPBadRequest.with_reason(req, "Handshake error"));
    }
    let key = {
        let key = req.headers().get(SEC_WEBSOCKET_KEY).unwrap();
        hash_key(key.as_ref())
    };

    Ok(HttpResponse::new(req, StatusCode::SWITCHING_PROTOCOLS, Body::Empty)
       .set_connection_type(ConnectionType::Upgrade)
       .set_header(header::UPGRADE, header::HeaderValue::from_static("websocket"))
       .set_header(header::TRANSFER_ENCODING, header::HeaderValue::from_static("chunked"))
       .set_header(header::HeaderName::from_str(SEC_WEBSOCKET_ACCEPT).unwrap(),
                   header::HeaderValue::from_str(key.as_str()).unwrap())
       .set_body(Body::Upgrade)
    )
}


/// Maps `Payload` stream into stream of `ws::Message` items
pub struct WsStream {
    rx: Payload,
    buf: BytesMut,
}

impl WsStream {
    pub fn new(rx: Payload) -> WsStream {
        WsStream { rx: rx, buf: BytesMut::new() }
    }
}

impl Stream for WsStream {
    type Item = Message;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let mut done = false;

        loop {
            match self.rx.readany() {
                Async::Ready(Some(chunk)) => {
                    self.buf.extend(chunk)
                }
                Async::Ready(None) => {
                    done = true;
                }
                Async::NotReady => break,
            }
        }

        loop {
            match wsframe::Frame::parse(&mut self.buf) {
                Ok(Some(frame)) => {
                    trace!("Frame {}", frame);
                    let (_finished, opcode, payload) = frame.unpack();

                    match opcode {
                        OpCode::Continue => continue,
                        OpCode::Bad =>
                            return Ok(Async::Ready(Some(Message::Error))),
                        OpCode::Close =>
                            return Ok(Async::Ready(Some(Message::Closed))),
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
                Ok(None) => if done {
                    return Ok(Async::Ready(None))
                } else {
                    return Ok(Async::NotReady)
                },
                Err(_) =>
                    return Err(()),
            }
        }
    }
}


/// `WebSocket` writer
pub struct WsWriter;

impl WsWriter {

    /// Send text frame
    pub fn text<A>(ctx: &mut HttpContext<A>, text: String)
        where A: Actor<Context=HttpContext<A>> + Route
    {
        let mut frame = wsframe::Frame::message(Vec::from(text.as_str()), OpCode::Text, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        ctx.write(
            Bytes::from(buf.as_slice())
        );
    }

    /// Send binary frame
    pub fn binary<A>(ctx: &mut HttpContext<A>, data: Vec<u8>)
        where A: Actor<Context=HttpContext<A>> + Route
    {
        let mut frame = wsframe::Frame::message(data, OpCode::Binary, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        ctx.write(
            Bytes::from(buf.as_slice())
        );
    }

    /// Send ping frame
    pub fn ping<A>(ctx: &mut HttpContext<A>, message: String)
        where A: Actor<Context=HttpContext<A>> + Route
    {
        let mut frame = wsframe::Frame::message(
            Vec::from(message.as_str()), OpCode::Ping, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        ctx.write(
            Bytes::from(buf.as_slice())
        )
    }

    /// Send pong frame
    pub fn pong<A>(ctx: &mut HttpContext<A>, message: String)
        where A: Actor<Context=HttpContext<A>> + Route
    {
        let mut frame = wsframe::Frame::message(
            Vec::from(message.as_str()), OpCode::Pong, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        ctx.write(
            Bytes::from(buf.as_slice())
        )
    }
}
