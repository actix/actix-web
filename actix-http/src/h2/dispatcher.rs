use std::collections::VecDeque;
use std::marker::PhantomData;
use std::time::Instant;
use std::{fmt, mem, net};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_server_config::IoStream;
use actix_service::Service;
use bitflags::bitflags;
use bytes::{Bytes, BytesMut};
use futures::{try_ready, Async, Future, Poll, Sink, Stream};
use h2::server::{Connection, SendResponse};
use h2::{RecvStream, SendStream};
use http::header::{
    HeaderValue, ACCEPT_ENCODING, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING,
};
use http::HttpTryFrom;
use log::{debug, error, trace};
use tokio_timer::Delay;

use crate::body::{Body, BodySize, MessageBody, ResponseBody};
use crate::cloneable::CloneableService;
use crate::config::ServiceConfig;
use crate::error::{DispatchError, Error, ParseError, PayloadError, ResponseError};
use crate::helpers::DataFactory;
use crate::httpmessage::HttpMessage;
use crate::message::ResponseHead;
use crate::payload::Payload;
use crate::request::Request;
use crate::response::Response;

const CHUNK_SIZE: usize = 16_384;

/// Dispatcher for HTTP/2 protocol
pub struct Dispatcher<T: IoStream, S: Service<Request = Request>, B: MessageBody> {
    service: CloneableService<S>,
    connection: Connection<T, Bytes>,
    on_connect: Option<Box<dyn DataFactory>>,
    config: ServiceConfig,
    peer_addr: Option<net::SocketAddr>,
    ka_expire: Instant,
    ka_timer: Option<Delay>,
    _t: PhantomData<B>,
}

impl<T, S, B> Dispatcher<T, S, B>
where
    T: IoStream,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    pub(crate) fn new(
        service: CloneableService<S>,
        connection: Connection<T, Bytes>,
        on_connect: Option<Box<dyn DataFactory>>,
        config: ServiceConfig,
        timeout: Option<Delay>,
        peer_addr: Option<net::SocketAddr>,
    ) -> Self {
        // let keepalive = config.keep_alive_enabled();
        // let flags = if keepalive {
        // Flags::KEEPALIVE | Flags::KEEPALIVE_ENABLED
        // } else {
        //     Flags::empty()
        // };

        // keep-alive timer
        let (ka_expire, ka_timer) = if let Some(delay) = timeout {
            (delay.deadline(), Some(delay))
        } else if let Some(delay) = config.keep_alive_timer() {
            (delay.deadline(), Some(delay))
        } else {
            (config.now(), None)
        };

        Dispatcher {
            service,
            config,
            peer_addr,
            connection,
            on_connect,
            ka_expire,
            ka_timer,
            _t: PhantomData,
        }
    }
}

impl<T, S, B> Future for Dispatcher<T, S, B>
where
    T: IoStream,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    type Item = ();
    type Error = DispatchError;

    #[inline]
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match self.connection.poll()? {
                Async::Ready(None) => return Ok(Async::Ready(())),
                Async::Ready(Some((req, res))) => {
                    // update keep-alive expire
                    if self.ka_timer.is_some() {
                        if let Some(expire) = self.config.keep_alive_expire() {
                            self.ka_expire = expire;
                        }
                    }

                    let (parts, body) = req.into_parts();
                    let mut req = Request::with_payload(body.into());

                    let head = &mut req.head_mut();
                    head.uri = parts.uri;
                    head.method = parts.method;
                    head.version = parts.version;
                    head.headers = parts.headers.into();
                    head.peer_addr = self.peer_addr;

                    // set on_connect data
                    if let Some(ref on_connect) = self.on_connect {
                        on_connect.set(&mut req.extensions_mut());
                    }

                    tokio_current_thread::spawn(ServiceResponse::<S::Future, B> {
                        state: ServiceResponseState::ServiceCall(
                            self.service.call(req),
                            Some(res),
                        ),
                        config: self.config.clone(),
                        buffer: None,
                    })
                }
                Async::NotReady => return Ok(Async::NotReady),
            }
        }
    }
}

struct ServiceResponse<F, B> {
    state: ServiceResponseState<F, B>,
    config: ServiceConfig,
    buffer: Option<Bytes>,
}

enum ServiceResponseState<F, B> {
    ServiceCall(F, Option<SendResponse<Bytes>>),
    SendPayload(SendStream<Bytes>, ResponseBody<B>),
}

impl<F, B> ServiceResponse<F, B>
where
    F: Future,
    F::Error: Into<Error>,
    F::Item: Into<Response<B>>,
    B: MessageBody + 'static,
{
    fn prepare_response(
        &self,
        head: &ResponseHead,
        size: &mut BodySize,
    ) -> http::Response<()> {
        let mut has_date = false;
        let mut skip_len = size != &BodySize::Stream;

        let mut res = http::Response::new(());
        *res.status_mut() = head.status;
        *res.version_mut() = http::Version::HTTP_2;

        // Content length
        match head.status {
            http::StatusCode::NO_CONTENT
            | http::StatusCode::CONTINUE
            | http::StatusCode::PROCESSING => *size = BodySize::None,
            http::StatusCode::SWITCHING_PROTOCOLS => {
                skip_len = true;
                *size = BodySize::Stream;
            }
            _ => (),
        }
        let _ = match size {
            BodySize::None | BodySize::Stream => None,
            BodySize::Empty => res
                .headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from_static("0")),
            BodySize::Sized(len) => res.headers_mut().insert(
                CONTENT_LENGTH,
                HeaderValue::try_from(format!("{}", len)).unwrap(),
            ),
            BodySize::Sized64(len) => res.headers_mut().insert(
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
            res.headers_mut().append(key, value.clone());
        }

        // set date header
        if !has_date {
            let mut bytes = BytesMut::with_capacity(29);
            self.config.set_date_header(&mut bytes);
            res.headers_mut()
                .insert(DATE, HeaderValue::try_from(bytes.freeze()).unwrap());
        }

        res
    }
}

impl<F, B> Future for ServiceResponse<F, B>
where
    F: Future,
    F::Error: Into<Error>,
    F::Item: Into<Response<B>>,
    B: MessageBody + 'static,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.state {
            ServiceResponseState::ServiceCall(ref mut call, ref mut send) => {
                match call.poll() {
                    Ok(Async::Ready(res)) => {
                        let (res, body) = res.into().replace_body(());

                        let mut send = send.take().unwrap();
                        let mut size = body.size();
                        let h2_res = self.prepare_response(res.head(), &mut size);

                        let stream =
                            send.send_response(h2_res, size.is_eof()).map_err(|e| {
                                trace!("Error sending h2 response: {:?}", e);
                            })?;

                        if size.is_eof() {
                            Ok(Async::Ready(()))
                        } else {
                            self.state = ServiceResponseState::SendPayload(stream, body);
                            self.poll()
                        }
                    }
                    Ok(Async::NotReady) => Ok(Async::NotReady),
                    Err(_e) => {
                        let res: Response = Response::InternalServerError().finish();
                        let (res, body) = res.replace_body(());

                        let mut send = send.take().unwrap();
                        let mut size = body.size();
                        let h2_res = self.prepare_response(res.head(), &mut size);

                        let stream =
                            send.send_response(h2_res, size.is_eof()).map_err(|e| {
                                trace!("Error sending h2 response: {:?}", e);
                            })?;

                        if size.is_eof() {
                            Ok(Async::Ready(()))
                        } else {
                            self.state = ServiceResponseState::SendPayload(
                                stream,
                                body.into_body(),
                            );
                            self.poll()
                        }
                    }
                }
            }
            ServiceResponseState::SendPayload(ref mut stream, ref mut body) => loop {
                loop {
                    if let Some(ref mut buffer) = self.buffer {
                        match stream.poll_capacity().map_err(|e| warn!("{:?}", e))? {
                            Async::NotReady => return Ok(Async::NotReady),
                            Async::Ready(None) => return Ok(Async::Ready(())),
                            Async::Ready(Some(cap)) => {
                                let len = buffer.len();
                                let bytes = buffer.split_to(std::cmp::min(cap, len));

                                if let Err(e) = stream.send_data(bytes, false) {
                                    warn!("{:?}", e);
                                    return Err(());
                                } else if !buffer.is_empty() {
                                    let cap = std::cmp::min(buffer.len(), CHUNK_SIZE);
                                    stream.reserve_capacity(cap);
                                } else {
                                    self.buffer.take();
                                }
                            }
                        }
                    } else {
                        match body.poll_next() {
                            Ok(Async::NotReady) => {
                                return Ok(Async::NotReady);
                            }
                            Ok(Async::Ready(None)) => {
                                if let Err(e) = stream.send_data(Bytes::new(), true) {
                                    warn!("{:?}", e);
                                    return Err(());
                                } else {
                                    return Ok(Async::Ready(()));
                                }
                            }
                            Ok(Async::Ready(Some(chunk))) => {
                                stream.reserve_capacity(std::cmp::min(
                                    chunk.len(),
                                    CHUNK_SIZE,
                                ));
                                self.buffer = Some(chunk);
                            }
                            Err(e) => {
                                error!("Response payload stream error: {:?}", e);
                                return Err(());
                            }
                        }
                    }
                }
            },
        }
    }
}
