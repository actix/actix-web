use std::task::{Context, Poll};
use std::{cmp, future::Future, marker::PhantomData, net, pin::Pin, rc::Rc};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::Service;
use bytes::{Bytes, BytesMut};
use futures_core::ready;
use h2::{
    server::{Connection, SendResponse},
    SendStream,
};
use http::header::{HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING};
use log::{error, trace};

use crate::body::{BodySize, MessageBody, ResponseBody};
use crate::config::ServiceConfig;
use crate::error::{DispatchError, Error};
use crate::message::ResponseHead;
use crate::payload::Payload;
use crate::request::Request;
use crate::response::Response;
use crate::service::HttpFlow;
use crate::OnConnectData;

const CHUNK_SIZE: usize = 16_384;

/// Dispatcher for HTTP/2 protocol.
#[pin_project::pin_project]
pub struct Dispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request>,
    B: MessageBody,
{
    flow: Rc<HttpFlow<S, X, U>>,
    connection: Connection<T, Bytes>,
    on_connect_data: OnConnectData,
    config: ServiceConfig,
    peer_addr: Option<net::SocketAddr>,
    _phantom: PhantomData<B>,
}

impl<T, S, B, X, U> Dispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    pub(crate) fn new(
        flow: Rc<HttpFlow<S, X, U>>,
        connection: Connection<T, Bytes>,
        on_connect_data: OnConnectData,
        config: ServiceConfig,
        peer_addr: Option<net::SocketAddr>,
    ) -> Self {
        Dispatcher {
            flow,
            config,
            peer_addr,
            connection,
            on_connect_data,
            _phantom: PhantomData,
        }
    }
}

impl<T, S, B, X, U> Future for Dispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request>,
    S::Error: Into<Error> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody + 'static,
{
    type Output = Result<(), DispatchError>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            match ready!(Pin::new(&mut this.connection).poll_accept(cx)) {
                None => return Poll::Ready(Ok(())),

                Some(Err(err)) => return Poll::Ready(Err(err.into())),

                Some(Ok((req, res))) => {
                    let (parts, body) = req.into_parts();
                    let pl = crate::h2::Payload::new(body);
                    let pl = Payload::<crate::payload::PayloadStream>::H2(pl);
                    let mut req = Request::with_payload(pl);

                    let head = req.head_mut();
                    head.uri = parts.uri;
                    head.method = parts.method;
                    head.version = parts.version;
                    head.headers = parts.headers.into();
                    head.peer_addr = this.peer_addr;

                    // merge on_connect_ext data into request extensions
                    this.on_connect_data.merge_into(&mut req);

                    let svc = ServiceResponse {
                        state: ServiceResponseState::ServiceCall(
                            this.flow.service.call(req),
                            Some(res),
                        ),
                        config: this.config.clone(),
                        buffer: None,
                        _phantom: PhantomData,
                    };

                    actix_rt::spawn(svc);
                }
            }
        }
    }
}

#[pin_project::pin_project]
struct ServiceResponse<F, I, E, B> {
    #[pin]
    state: ServiceResponseState<F, B>,
    config: ServiceConfig,
    buffer: Option<Bytes>,
    _phantom: PhantomData<(I, E)>,
}

#[pin_project::pin_project(project = ServiceResponseStateProj)]
enum ServiceResponseState<F, B> {
    ServiceCall(#[pin] F, Option<SendResponse<Bytes>>),
    SendPayload(SendStream<Bytes>, #[pin] ResponseBody<B>),
}

impl<F, I, E, B> ServiceResponse<F, I, E, B>
where
    F: Future<Output = Result<I, E>>,
    E: Into<Error>,
    I: Into<Response<B>>,
    B: MessageBody,
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
            _ => {}
        }

        let _ = match size {
            BodySize::None | BodySize::Stream => None,
            BodySize::Empty => res
                .headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from_static("0")),
            BodySize::Sized(len) => {
                let mut buf = itoa::Buffer::new();

                res.headers_mut().insert(
                    CONTENT_LENGTH,
                    HeaderValue::from_str(buf.format(*len)).unwrap(),
                )
            }
        };

        // copy headers
        for (key, value) in head.headers.iter() {
            match *key {
                // TODO: consider skipping other headers according to:
                //       https://tools.ietf.org/html/rfc7540#section-8.1.2.2
                // omit HTTP/1.x only headers
                CONNECTION | TRANSFER_ENCODING => continue,
                CONTENT_LENGTH if skip_len => continue,
                DATE => has_date = true,
                _ => {}
            }

            res.headers_mut().append(key, value.clone());
        }

        // set date header
        if !has_date {
            let mut bytes = BytesMut::with_capacity(29);
            self.config.set_date_header(&mut bytes);
            res.headers_mut().insert(
                DATE,
                // SAFETY: serialized date-times are known ASCII strings
                unsafe { HeaderValue::from_maybe_shared_unchecked(bytes.freeze()) },
            );
        }

        res
    }
}

impl<F, I, E, B> Future for ServiceResponse<F, I, E, B>
where
    F: Future<Output = Result<I, E>>,
    E: Into<Error>,
    I: Into<Response<B>>,
    B: MessageBody,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        match this.state.project() {
            ServiceResponseStateProj::ServiceCall(call, send) => {
                match ready!(call.poll(cx)) {
                    Ok(res) => {
                        let (res, body) = res.into().replace_body(());

                        let mut send = send.take().unwrap();
                        let mut size = body.size();
                        let h2_res =
                            self.as_mut().prepare_response(res.head(), &mut size);
                        this = self.as_mut().project();

                        let stream = match send.send_response(h2_res, size.is_eof()) {
                            Err(e) => {
                                trace!("Error sending HTTP/2 response: {:?}", e);
                                return Poll::Ready(());
                            }
                            Ok(stream) => stream,
                        };

                        if size.is_eof() {
                            Poll::Ready(())
                        } else {
                            this.state
                                .set(ServiceResponseState::SendPayload(stream, body));
                            self.poll(cx)
                        }
                    }

                    Err(err) => {
                        let res = Response::from_error(err.into());
                        let (res, body) = res.replace_body(());

                        let mut send = send.take().unwrap();
                        let mut size = body.size();
                        let h2_res =
                            self.as_mut().prepare_response(res.head(), &mut size);
                        this = self.as_mut().project();

                        let stream = match send.send_response(h2_res, size.is_eof()) {
                            Err(e) => {
                                trace!("Error sending HTTP/2 response: {:?}", e);
                                return Poll::Ready(());
                            }
                            Ok(stream) => stream,
                        };

                        if size.is_eof() {
                            Poll::Ready(())
                        } else {
                            this.state.set(ServiceResponseState::SendPayload(
                                stream,
                                body.into_body(),
                            ));
                            self.poll(cx)
                        }
                    }
                }
            }

            ServiceResponseStateProj::SendPayload(ref mut stream, ref mut body) => {
                loop {
                    match this.buffer {
                        Some(ref mut buffer) => match ready!(stream.poll_capacity(cx)) {
                            None => return Poll::Ready(()),

                            Some(Ok(cap)) => {
                                let len = buffer.len();
                                let bytes = buffer.split_to(cmp::min(cap, len));

                                if let Err(e) = stream.send_data(bytes, false) {
                                    warn!("{:?}", e);
                                    return Poll::Ready(());
                                } else if !buffer.is_empty() {
                                    let cap = cmp::min(buffer.len(), CHUNK_SIZE);
                                    stream.reserve_capacity(cap);
                                } else {
                                    this.buffer.take();
                                }
                            }

                            Some(Err(e)) => {
                                warn!("{:?}", e);
                                return Poll::Ready(());
                            }
                        },

                        None => match ready!(body.as_mut().poll_next(cx)) {
                            None => {
                                if let Err(e) = stream.send_data(Bytes::new(), true) {
                                    warn!("{:?}", e);
                                }
                                return Poll::Ready(());
                            }

                            Some(Ok(chunk)) => {
                                stream
                                    .reserve_capacity(cmp::min(chunk.len(), CHUNK_SIZE));
                                *this.buffer = Some(chunk);
                            }

                            Some(Err(e)) => {
                                error!("Response payload stream error: {:?}", e);
                                return Poll::Ready(());
                            }
                        },
                    }
                }
            }
        }
    }
}
