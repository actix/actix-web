use std::io::Write;
use std::{io, time};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use bytes::{BufMut, Bytes, BytesMut};
use futures::future::{ok, Either};
use futures::{Async, Future, Poll, Sink, Stream};

use crate::error::PayloadError;
use crate::h1;
use crate::http::header::{IntoHeaderValue, HOST};
use crate::message::{RequestHead, ResponseHead};
use crate::payload::{Payload, PayloadStream};

use super::connection::{ConnectionLifetime, ConnectionType, IoConnection};
use super::error::{ConnectError, SendRequestError};
use super::pool::Acquired;
use crate::body::{BodySize, MessageBody};

pub(crate) fn send_request<T, B>(
    io: T,
    mut head: RequestHead,
    body: B,
    created: time::Instant,
    pool: Option<Acquired<T>>,
) -> impl Future<Item = (ResponseHead, Payload), Error = SendRequestError>
where
    T: AsyncRead + AsyncWrite + 'static,
    B: MessageBody,
{
    // set request host header
    if !head.headers.contains_key(HOST) {
        if let Some(host) = head.uri.host() {
            let mut wrt = BytesMut::with_capacity(host.len() + 5).writer();

            let _ = match head.uri.port_u16() {
                None | Some(80) | Some(443) => write!(wrt, "{}", host),
                Some(port) => write!(wrt, "{}:{}", host, port),
            };

            match wrt.get_mut().take().freeze().try_into() {
                Ok(value) => {
                    head.headers.insert(HOST, value);
                }
                Err(e) => {
                    log::error!("Can not set HOST header {}", e);
                }
            }
        }
    }

    let io = H1Connection {
        created,
        pool,
        io: Some(io),
    };

    let len = body.size();

    // create Framed and send reqest
    Framed::new(io, h1::ClientCodec::default())
        .send((head, len).into())
        .from_err()
        // send request body
        .and_then(move |framed| match body.size() {
            BodySize::None | BodySize::Empty | BodySize::Sized(0) => {
                Either::A(ok(framed))
            }
            _ => Either::B(SendBody::new(body, framed)),
        })
        // read response and init read body
        .and_then(|framed| {
            framed
                .into_future()
                .map_err(|(e, _)| SendRequestError::from(e))
                .and_then(|(item, framed)| {
                    if let Some(res) = item {
                        match framed.get_codec().message_type() {
                            h1::MessageType::None => {
                                let force_close = !framed.get_codec().keepalive();
                                release_connection(framed, force_close);
                                Ok((res, Payload::None))
                            }
                            _ => {
                                let pl: PayloadStream = Box::new(PlStream::new(framed));
                                Ok((res, pl.into()))
                            }
                        }
                    } else {
                        Err(ConnectError::Disconnected.into())
                    }
                })
        })
}

pub(crate) fn open_tunnel<T>(
    io: T,
    head: RequestHead,
) -> impl Future<Item = (ResponseHead, Framed<T, h1::ClientCodec>), Error = SendRequestError>
where
    T: AsyncRead + AsyncWrite + 'static,
{
    // create Framed and send reqest
    Framed::new(io, h1::ClientCodec::default())
        .send((head, BodySize::None).into())
        .from_err()
        // read response
        .and_then(|framed| {
            framed
                .into_future()
                .map_err(|(e, _)| SendRequestError::from(e))
                .and_then(|(head, framed)| {
                    if let Some(head) = head {
                        Ok((head, framed))
                    } else {
                        Err(SendRequestError::from(ConnectError::Disconnected))
                    }
                })
        })
}

#[doc(hidden)]
/// HTTP client connection
pub struct H1Connection<T> {
    io: Option<T>,
    created: time::Instant,
    pool: Option<Acquired<T>>,
}

impl<T: AsyncRead + AsyncWrite + 'static> ConnectionLifetime for H1Connection<T> {
    /// Close connection
    fn close(&mut self) {
        if let Some(mut pool) = self.pool.take() {
            if let Some(io) = self.io.take() {
                pool.close(IoConnection::new(
                    ConnectionType::H1(io),
                    self.created,
                    None,
                ));
            }
        }
    }

    /// Release this connection to the connection pool
    fn release(&mut self) {
        if let Some(mut pool) = self.pool.take() {
            if let Some(io) = self.io.take() {
                pool.release(IoConnection::new(
                    ConnectionType::H1(io),
                    self.created,
                    None,
                ));
            }
        }
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> io::Read for H1Connection<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.io.as_mut().unwrap().read(buf)
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> AsyncRead for H1Connection<T> {}

impl<T: AsyncRead + AsyncWrite + 'static> io::Write for H1Connection<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.io.as_mut().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.io.as_mut().unwrap().flush()
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> AsyncWrite for H1Connection<T> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.io.as_mut().unwrap().shutdown()
    }
}

/// Future responsible for sending request body to the peer
pub(crate) struct SendBody<I, B> {
    body: Option<B>,
    framed: Option<Framed<I, h1::ClientCodec>>,
    flushed: bool,
}

impl<I, B> SendBody<I, B>
where
    I: AsyncRead + AsyncWrite + 'static,
    B: MessageBody,
{
    pub(crate) fn new(body: B, framed: Framed<I, h1::ClientCodec>) -> Self {
        SendBody {
            body: Some(body),
            framed: Some(framed),
            flushed: true,
        }
    }
}

impl<I, B> Future for SendBody<I, B>
where
    I: ConnectionLifetime,
    B: MessageBody,
{
    type Item = Framed<I, h1::ClientCodec>;
    type Error = SendRequestError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut body_ready = true;
        loop {
            while body_ready
                && self.body.is_some()
                && !self.framed.as_ref().unwrap().is_write_buf_full()
            {
                match self.body.as_mut().unwrap().poll_next()? {
                    Async::Ready(item) => {
                        // check if body is done
                        if item.is_none() {
                            let _ = self.body.take();
                        }
                        self.flushed = false;
                        self.framed
                            .as_mut()
                            .unwrap()
                            .force_send(h1::Message::Chunk(item))?;
                        break;
                    }
                    Async::NotReady => body_ready = false,
                }
            }

            if !self.flushed {
                match self.framed.as_mut().unwrap().poll_complete()? {
                    Async::Ready(_) => {
                        self.flushed = true;
                        continue;
                    }
                    Async::NotReady => return Ok(Async::NotReady),
                }
            }

            if self.body.is_none() {
                return Ok(Async::Ready(self.framed.take().unwrap()));
            }
            return Ok(Async::NotReady);
        }
    }
}

pub(crate) struct PlStream<Io> {
    framed: Option<Framed<Io, h1::ClientPayloadCodec>>,
}

impl<Io: ConnectionLifetime> PlStream<Io> {
    fn new(framed: Framed<Io, h1::ClientCodec>) -> Self {
        PlStream {
            framed: Some(framed.map_codec(|codec| codec.into_payload_codec())),
        }
    }
}

impl<Io: ConnectionLifetime> Stream for PlStream<Io> {
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.framed.as_mut().unwrap().poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(Some(chunk)) => {
                if let Some(chunk) = chunk {
                    Ok(Async::Ready(Some(chunk)))
                } else {
                    let framed = self.framed.take().unwrap();
                    let force_close = !framed.get_codec().keepalive();
                    release_connection(framed, force_close);
                    Ok(Async::Ready(None))
                }
            }
            Async::Ready(None) => Ok(Async::Ready(None)),
        }
    }
}

fn release_connection<T, U>(framed: Framed<T, U>, force_close: bool)
where
    T: ConnectionLifetime,
{
    let mut parts = framed.into_parts();
    if !force_close && parts.read_buf.is_empty() && parts.write_buf.is_empty() {
        parts.io.release()
    } else {
        parts.io.close()
    }
}
