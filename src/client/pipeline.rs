use std::collections::VecDeque;

use actix_net::codec::Framed;
use actix_net::service::Service;
use bytes::Bytes;
use futures::future::{err, ok, Either};
use futures::{Async, Future, Poll, Sink, Stream};
use tokio_io::{AsyncRead, AsyncWrite};

use super::error::{ConnectorError, SendRequestError};
use super::response::ClientResponse;
use super::{Connect, Connection};
use crate::body::{BodyLength, MessageBody, PayloadStream};
use crate::error::PayloadError;
use crate::h1;
use crate::message::RequestHead;

pub(crate) fn send_request<T, I, B>(
    head: RequestHead,
    body: B,
    connector: &mut T,
) -> impl Future<Item = ClientResponse, Error = SendRequestError>
where
    T: Service<Connect, Response = I, Error = ConnectorError>,
    B: MessageBody,
    I: Connection,
{
    let len = body.length();

    connector
        // connect to the host
        .call(Connect::new(head.uri.clone()))
        .from_err()
        // create Framed and send reqest
        .map(|io| Framed::new(io, h1::ClientCodec::default()))
        .and_then(move |framed| framed.send((head, len).into()).from_err())
        // send request body
        .and_then(move |framed| match body.length() {
            BodyLength::None | BodyLength::Empty | BodyLength::Sized(0) => {
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
                                release_connection(framed, force_close)
                            }
                            _ => {
                                *res.payload.borrow_mut() = Some(Payload::stream(framed))
                            }
                        }
                        ok(res)
                    } else {
                        err(ConnectorError::Disconnected.into())
                    }
                })
        })
}

/// Future responsible for sending request body to the peer
struct SendBody<I, B> {
    body: Option<B>,
    framed: Option<Framed<I, h1::ClientCodec>>,
    write_buf: VecDeque<h1::Message<(RequestHead, BodyLength)>>,
    flushed: bool,
}

impl<I, B> SendBody<I, B>
where
    I: AsyncRead + AsyncWrite + 'static,
    B: MessageBody,
{
    fn new(body: B, framed: Framed<I, h1::ClientCodec>) -> Self {
        SendBody {
            body: Some(body),
            framed: Some(framed),
            write_buf: VecDeque::new(),
            flushed: true,
        }
    }
}

impl<I, B> Future for SendBody<I, B>
where
    I: Connection,
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

struct EmptyPayload;

impl Stream for EmptyPayload {
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        Ok(Async::Ready(None))
    }
}

pub(crate) struct Payload<Io> {
    framed: Option<Framed<Io, h1::ClientPayloadCodec>>,
}

impl Payload<()> {
    pub fn empty() -> PayloadStream {
        Box::new(EmptyPayload)
    }
}

impl<Io: Connection> Payload<Io> {
    fn stream(framed: Framed<Io, h1::ClientCodec>) -> PayloadStream {
        Box::new(Payload {
            framed: Some(framed.map_codec(|codec| codec.into_payload_codec())),
        })
    }
}

impl<Io: Connection> Stream for Payload<Io> {
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
                    let force_close = framed.get_codec().keepalive();
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
    T: Connection,
{
    let mut parts = framed.into_parts();
    if !force_close && parts.read_buf.is_empty() && parts.write_buf.is_empty() {
        parts.io.release()
    } else {
        parts.io.close()
    }
}
