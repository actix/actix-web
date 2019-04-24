use std::time;

use actix_codec::{AsyncRead, AsyncWrite};
use bytes::Bytes;
use futures::future::{err, Either};
use futures::{Async, Future, Poll};
use h2::{client::SendRequest, SendStream};
use http::header::{HeaderValue, CONNECTION, CONTENT_LENGTH, TRANSFER_ENCODING};
use http::{request::Request, HttpTryFrom, Method, Version};

use crate::body::{BodySize, MessageBody};
use crate::message::{RequestHead, ResponseHead};
use crate::payload::Payload;

use super::connection::{ConnectionType, IoConnection};
use super::error::SendRequestError;
use super::pool::Acquired;

pub(crate) fn send_request<T, B>(
    io: SendRequest<Bytes>,
    head: RequestHead,
    body: B,
    created: time::Instant,
    pool: Option<Acquired<T>>,
) -> impl Future<Item = (ResponseHead, Payload), Error = SendRequestError>
where
    T: AsyncRead + AsyncWrite + 'static,
    B: MessageBody,
{
    trace!("Sending client request: {:?} {:?}", head, body.size());
    let head_req = head.method == Method::HEAD;
    let length = body.size();
    let eof = match length {
        BodySize::None | BodySize::Empty | BodySize::Sized(0) => true,
        _ => false,
    };

    io.ready()
        .map_err(SendRequestError::from)
        .and_then(move |mut io| {
            let mut req = Request::new(());
            *req.uri_mut() = head.uri;
            *req.method_mut() = head.method;
            *req.version_mut() = Version::HTTP_2;

            let mut skip_len = true;
            // let mut has_date = false;

            // Content length
            let _ = match length {
                BodySize::None => None,
                BodySize::Stream => {
                    skip_len = false;
                    None
                }
                BodySize::Empty => req
                    .headers_mut()
                    .insert(CONTENT_LENGTH, HeaderValue::from_static("0")),
                BodySize::Sized(len) => req.headers_mut().insert(
                    CONTENT_LENGTH,
                    HeaderValue::try_from(format!("{}", len)).unwrap(),
                ),
                BodySize::Sized64(len) => req.headers_mut().insert(
                    CONTENT_LENGTH,
                    HeaderValue::try_from(format!("{}", len)).unwrap(),
                ),
            };

            // copy headers
            for (key, value) in head.headers.iter() {
                match *key {
                    CONNECTION | TRANSFER_ENCODING => continue, // http2 specific
                    CONTENT_LENGTH if skip_len => continue,
                    // DATE => has_date = true,
                    _ => (),
                }
                req.headers_mut().append(key, value.clone());
            }

            match io.send_request(req, eof) {
                Ok((res, send)) => {
                    release(io, pool, created, false);

                    if !eof {
                        Either::A(Either::B(
                            SendBody {
                                body,
                                send,
                                buf: None,
                            }
                            .and_then(move |_| res.map_err(SendRequestError::from)),
                        ))
                    } else {
                        Either::B(res.map_err(SendRequestError::from))
                    }
                }
                Err(e) => {
                    release(io, pool, created, e.is_io());
                    Either::A(Either::A(err(e.into())))
                }
            }
        })
        .and_then(move |resp| {
            let (parts, body) = resp.into_parts();
            let payload = if head_req { Payload::None } else { body.into() };

            let mut head = ResponseHead::new(parts.status);
            head.version = parts.version;
            head.headers = parts.headers.into();
            Ok((head, payload))
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
        loop {
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
                        continue;
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
