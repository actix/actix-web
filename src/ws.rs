//! `WebSocket` context implementation

#![allow(dead_code, unused_variables, unused_imports)]

use std::io;
use std::vec::Vec;
use std::borrow::Cow;

use http::{Method, StatusCode};
use bytes::{Bytes, BytesMut};
use futures::{Async, Future, Poll, Stream};
use hyper::header;

use actix::Actor;

use context::HttpContext;
use route::{Route, Payload, PayloadItem};
use httpcodes::{HTTPBadRequest, HTTPMethodNotAllowed};
use httpmessage::{Body, ConnectionType, HttpRequest, HttpResponse, IntoHttpResponse};

use wsframe;
pub use wsproto::*;

header! { (WebSocketAccept, "SEC-WEBSOCKET-ACCEPT") => [String] }
header! { (WebSocketKey, "SEC-WEBSOCKET-KEY") => [String] }
header! { (WebSocketVersion, "SEC-WEBSOCKET-VERSION") => [String] }
header! { (WebSocketProtocol, "SEC-WEBSOCKET-PROTOCOL") => [String] }


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

#[derive(Debug)]
pub enum SendMessage {
    Text(String),
    Binary(Vec<u8>),
    Close(CloseCode),
    Ping,
    Pong,
}

/// Prepare `WebSocket` handshake.
///
/// It return HTTP response code, response headers, websocket parser,
/// websocket writer. It does not perform any IO.
///
/// `protocols` is a sequence of known protocols. On successful handshake,
/// the returned response headers contain the first protocol in this list
/// which the server also knows.
pub fn do_handshake(req: HttpRequest) -> Result<HttpResponse, HttpResponse> {
    // WebSocket accepts only GET
    if *req.method() != Method::GET {
        return Err(HTTPMethodNotAllowed.response(req))
    }

    // Check for "UPGRADE" to websocket header
    let has_hdr = if let Some::<&header::Upgrade>(hdr) = req.headers().get() {
        hdr.0.contains(&header::Protocol::new(header::ProtocolName::WebSocket, None))
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
    if !req.headers().has::<WebSocketVersion>() {
        return Err(HTTPBadRequest.with_reason(req, "No websocket version header is required"))
    }
    let supported_ver = {
        let hdr = req.headers().get::<WebSocketVersion>().unwrap();
        match hdr.0.as_str() {
            "13" | "8" | "7"  => true,
            _ => false,
        }
    };
    if !supported_ver {
        return Err(HTTPBadRequest.with_reason(req, "Unsupported version"))
    }

    // check client handshake for validity
    let key = if let Some::<&WebSocketKey>(hdr) = req.headers().get() {
        Some(hash_key(hdr.0.as_bytes()))
    } else {
        None
    };
    let key = if let Some(key) = key {
        key
    } else {
        return Err(HTTPBadRequest.with_reason(req, "Handshake error"));
    };

    Ok(HttpResponse::new(req, StatusCode::SWITCHING_PROTOCOLS, Body::Empty)
       .set_connection_type(ConnectionType::Upgrade)
       .set_header(
           header::Upgrade(vec![header::Protocol::new(header::ProtocolName::WebSocket, None)]))
       .set_header(
           header::TransferEncoding(vec![header::Encoding::Chunked]))
       .set_header(
           WebSocketAccept(key))
       .set_body(Body::Upgrade)
    )
}


/// Struct represent stream of `ws::Message` items
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
            match self.rx.poll() {
                Ok(Async::Ready(Some(item))) => {
                    match item {
                        PayloadItem::Eof =>
                            return Ok(Async::Ready(None)),
                        PayloadItem::Chunk(chunk) => {
                            self.buf.extend(chunk)
                        }
                    }
                }
                Ok(Async::Ready(None)) => done = true,
                Ok(Async::NotReady) => {},
                Err(err) => return Err(err),
            }

            match wsframe::Frame::parse(&mut self.buf) {
                Ok(Some(frame)) => {
                    trace!("Frame {}", frame);
                    let (finished, opcode, payload) = frame.unpack();

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
                                Err(err) =>
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
                Err(err) =>
                    return Err(()),
            }
        }
    }
}


/// `WebSocket` writer
pub struct WsWriter;

impl WsWriter {

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

    pub fn ping<A>(ctx: &mut HttpContext<A>, message: String)
        where A: Actor<Context=HttpContext<A>> + Route
    {
        let mut frame = wsframe::Frame::ping(Vec::from(message.as_str()));
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        ctx.write(
            Bytes::from(buf.as_slice())
        )
    }

    pub fn pong<A>(ctx: &mut HttpContext<A>, message: String)
        where A: Actor<Context=HttpContext<A>> + Route
    {
        let mut frame = wsframe::Frame::pong(Vec::from(message.as_str()));
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        ctx.write(
            Bytes::from(buf.as_slice())
        )
    }
}
