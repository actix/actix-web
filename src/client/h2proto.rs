use std::cell::RefCell;
use std::time;

use actix_codec::{AsyncRead, AsyncWrite};
use bytes::Bytes;
use futures::future::{err, Either};
use futures::{Async, Future, Poll, Stream};
use h2::{client::SendRequest, SendStream};
use http::{request::Request, Version};

use super::connection::{ConnectionType, IoConnection};
use super::error::SendRequestError;
use super::pool::Acquired;
use super::response::ClientResponse;
use crate::body::{BodyLength, MessageBody};
use crate::message::{RequestHead, ResponseHead};

pub(crate) fn send_request<T, B>(
    io: SendRequest<Bytes>,
    head: RequestHead,
    body: B,
    created: time::Instant,
    pool: Option<Acquired<T>>,
) -> impl Future<Item = ClientResponse, Error = SendRequestError>
where
    T: AsyncRead + AsyncWrite + 'static,
    B: MessageBody,
{
    trace!("Sending client request: {:?} {:?}", head, body.length());
    let eof = match body.length() {
        BodyLength::None | BodyLength::Empty | BodyLength::Sized(0) => true,
        _ => false,
    };

    io.ready()
        .map_err(SendRequestError::from)
        .and_then(move |mut io| {
            let mut req = Request::new(());
            *req.uri_mut() = head.uri;
            *req.method_mut() = head.method;
            *req.headers_mut() = head.headers;
            *req.version_mut() = Version::HTTP_2;

            match io.send_request(req, eof) {
                Ok((resp, send)) => {
                    release(io, pool, created, false);

                    if !eof {
                        Either::A(Either::B(
                            SendBody {
                                body,
                                send,
                                buf: None,
                            }
                            .and_then(move |_| resp.map_err(SendRequestError::from)),
                        ))
                    } else {
                        Either::B(resp.map_err(SendRequestError::from))
                    }
                }
                Err(e) => {
                    release(io, pool, created, e.is_io());
                    Either::A(Either::A(err(e.into())))
                }
            }
        })
        .and_then(|resp| {
            let (parts, body) = resp.into_parts();

            let mut head = ResponseHead::default();
            head.version = parts.version;
            head.status = parts.status;
            head.headers = parts.headers;

            Ok(ClientResponse {
                head,
                payload: RefCell::new(Some(Box::new(body.from_err()))),
            })
        })
        .from_err()
}

struct SendBody<B: MessageBody> {
    body: B,
    send: SendStream<Bytes>,
    buf: Option<Bytes>,
}

impl<B: MessageBody> Future for SendBody<B> {
    type Item = ();
    type Error = SendRequestError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.buf.is_none() {
            match self.body.poll_next() {
                Ok(Async::Ready(Some(buf))) => {
                    self.send.reserve_capacity(buf.len());
                    self.buf = Some(buf);
                }
                Ok(Async::Ready(None)) => {
                    if let Err(e) = self.send.send_data(Bytes::new(), true) {
                        return Err(e.into());
                    }
                    self.send.reserve_capacity(0);
                    return Ok(Async::Ready(()));
                }
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(e) => return Err(e.into()),
            }
        }

        loop {
            match self.send.poll_capacity() {
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Ok(Async::Ready(None)) => return Ok(Async::Ready(())),
                Ok(Async::Ready(Some(cap))) => {
                    let mut buf = self.buf.take().unwrap();
                    let len = buf.len();
                    let bytes = buf.split_to(std::cmp::min(cap, len));

                    if let Err(e) = self.send.send_data(bytes, false) {
                        return Err(e.into());
                    } else {
                        if !buf.is_empty() {
                            self.send.reserve_capacity(buf.len());
                            self.buf = Some(buf);
                        }
                        return self.poll();
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

// release SendRequest object
fn release<T: AsyncRead + AsyncWrite + 'static>(
    io: SendRequest<Bytes>,
    pool: Option<Acquired<T>>,
    created: time::Instant,
    close: bool,
) {
    if let Some(mut pool) = pool {
        if close {
            pool.close(IoConnection::new(ConnectionType::H2(io), created, None));
        } else {
            pool.release(IoConnection::new(ConnectionType::H2(io), created, None));
        }
    }
}
