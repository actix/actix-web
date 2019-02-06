use std::cell::RefCell;
use std::time;

use actix_codec::{AsyncRead, AsyncWrite};
use bytes::Bytes;
use futures::future::{err, Either};
use futures::{Async, Future, Poll};
use h2::{client::SendRequest, SendStream};
use http::header::{HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING};
use http::{request::Request, HttpTryFrom, Version};

use crate::body::{BodyLength, MessageBody};
use crate::h2::Payload;
use crate::message::{RequestHead, ResponseHead};

use super::connection::{ConnectionType, IoConnection};
use super::error::SendRequestError;
use super::pool::Acquired;
use super::response::ClientResponse;

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
    let length = body.length();
    let eof = match length {
        BodyLength::None | BodyLength::Empty | BodyLength::Sized(0) => true,
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
            let mut has_date = false;

            // Content length
            let _ = match length {
                BodyLength::Chunked | BodyLength::None => None,
                BodyLength::Stream => {
                    skip_len = false;
                    None
                }
                BodyLength::Empty => req
                    .headers_mut()
                    .insert(CONTENT_LENGTH, HeaderValue::from_static("0")),
                BodyLength::Sized(len) => req.headers_mut().insert(
                    CONTENT_LENGTH,
                    HeaderValue::try_from(format!("{}", len)).unwrap(),
                ),
                BodyLength::Sized64(len) => req.headers_mut().insert(
                    CONTENT_LENGTH,
                    HeaderValue::try_from(format!("{}", len)).unwrap(),
                ),
            };

            // copy headers
            for (key, value) in head.headers.iter() {
                match *key {
                    CONNECTION | TRANSFER_ENCODING => continue, // http2 specific
                    CONTENT_LENGTH if skip_len => continue,
                    DATE => has_date = true,
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
        .and_then(|resp| {
            let (parts, body) = resp.into_parts();

            let mut head = ResponseHead::default();
            head.version = parts.version;
            head.status = parts.status;
            head.headers = parts.headers;

            Ok(ClientResponse {
                head,
                payload: RefCell::new(Some(Box::new(Payload::new(body)))),
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
