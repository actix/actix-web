use std::{
    cmp,
    error::Error as StdError,
    future::Future,
    marker::PhantomData,
    net,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_rt::time::Sleep;
use actix_service::Service;
use actix_utils::future::poll_fn;
use bytes::{Bytes, BytesMut};
use futures_core::ready;
use h2::{
    server::{Connection, SendResponse},
    Ping, PingPong,
};
use http::header::{HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING};
use log::{error, trace};
use pin_project_lite::pin_project;

use crate::{
    body::{AnyBody, BodySize, MessageBody},
    config::ServiceConfig,
    service::HttpFlow,
    OnConnectData, Payload, Request, Response, ResponseHead,
};

const CHUNK_SIZE: usize = 16_384;

pin_project! {
    /// Dispatcher for HTTP/2 protocol.
    pub struct Dispatcher<T, S, B, X, U> {
        flow: Rc<HttpFlow<S, X, U>>,
        connection: Connection<T, Bytes>,
        on_connect_data: OnConnectData,
        config: ServiceConfig,
        peer_addr: Option<net::SocketAddr>,
        ping_pong: Option<H2PingPong>,
        _phantom: PhantomData<B>
    }
}

impl<T, S, B, X, U> Dispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub(crate) fn new(
        flow: Rc<HttpFlow<S, X, U>>,
        mut connection: Connection<T, Bytes>,
        on_connect_data: OnConnectData,
        config: ServiceConfig,
        peer_addr: Option<net::SocketAddr>,
    ) -> Self {
        let ping_pong = config.keep_alive_timer().map(|timer| H2PingPong {
            timer: Box::pin(timer),
            on_flight: false,
            ping_pong: connection.ping_pong().unwrap(),
        });

        Self {
            flow,
            config,
            peer_addr,
            connection,
            on_connect_data,
            ping_pong,
            _phantom: PhantomData,
        }
    }
}

struct H2PingPong {
    timer: Pin<Box<Sleep>>,
    on_flight: bool,
    ping_pong: PingPong,
}

impl<T, S, B, X, U> Future for Dispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,

    S: Service<Request>,
    S::Error: Into<Response<AnyBody>>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,

    B: MessageBody,
    B::Error: Into<Box<dyn StdError>>,
{
    type Output = Result<(), crate::error::DispatchError>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            match Pin::new(&mut this.connection).poll_accept(cx)? {
                Poll::Ready(Some((req, tx))) => {
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

                    let fut = this.flow.service.call(req);
                    let config = this.config.clone();

                    // multiplex request handling with spawn task
                    actix_rt::spawn(async move {
                        // resolve service call and send response.
                        let res = match fut.await {
                            Ok(res) => handle_response(res.into(), tx, config).await,
                            Err(err) => {
                                let res: Response<AnyBody> = err.into();
                                handle_response(res, tx, config).await
                            }
                        };

                        // log error.
                        if let Err(err) = res {
                            match err {
                                DispatchError::SendResponse(err) => {
                                    trace!("Error sending HTTP/2 response: {:?}", err)
                                }
                                DispatchError::SendData(err) => warn!("{:?}", err),
                                DispatchError::ResponseBody(err) => {
                                    error!("Response payload stream error: {:?}", err)
                                }
                            }
                        }
                    });
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => match this.ping_pong.as_mut() {
                    Some(ping_pong) => loop {
                        if ping_pong.on_flight {
                            // When have on flight ping pong. poll pong and and keep alive timer.
                            // on success pong received update keep alive timer to determine the next timing of
                            // ping pong.
                            match ping_pong.ping_pong.poll_pong(cx)? {
                                Poll::Ready(_) => {
                                    ping_pong.on_flight = false;

                                    let dead_line =
                                        this.config.keep_alive_expire().unwrap();
                                    ping_pong.timer.as_mut().reset(dead_line);
                                }
                                Poll::Pending => {
                                    return ping_pong
                                        .timer
                                        .as_mut()
                                        .poll(cx)
                                        .map(|_| Ok(()))
                                }
                            }
                        } else {
                            // When there is no on flight ping pong. keep alive timer is used to wait for next
                            // timing of ping pong. Therefore at this point it serves as an interval instead.
                            ready!(ping_pong.timer.as_mut().poll(cx));

                            ping_pong.ping_pong.send_ping(Ping::opaque())?;

                            let dead_line = this.config.keep_alive_expire().unwrap();
                            ping_pong.timer.as_mut().reset(dead_line);

                            ping_pong.on_flight = true;
                        }
                    },
                    None => return Poll::Pending,
                },
            }
        }
    }
}

enum DispatchError {
    SendResponse(h2::Error),
    SendData(h2::Error),
    ResponseBody(Box<dyn StdError>),
}

async fn handle_response<B>(
    res: Response<B>,
    mut tx: SendResponse<Bytes>,
    config: ServiceConfig,
) -> Result<(), DispatchError>
where
    B: MessageBody,
    B::Error: Into<Box<dyn StdError>>,
{
    let (res, body) = res.replace_body(());

    // prepare response.
    let mut size = body.size();
    let res = prepare_response(config, res.head(), &mut size);
    let eof = size.is_eof();

    // send response head and return on eof.
    let mut stream = tx
        .send_response(res, eof)
        .map_err(DispatchError::SendResponse)?;

    if eof {
        return Ok(());
    }

    // poll response body and send chunks to client.
    actix_rt::pin!(body);

    while let Some(res) = poll_fn(|cx| body.as_mut().poll_next(cx)).await {
        let mut chunk = res.map_err(|err| DispatchError::ResponseBody(err.into()))?;

        'send: loop {
            // reserve enough space and wait for stream ready.
            stream.reserve_capacity(cmp::min(chunk.len(), CHUNK_SIZE));

            match poll_fn(|cx| stream.poll_capacity(cx)).await {
                // No capacity left. drop body and return.
                None => return Ok(()),
                Some(res) => {
                    // Split chuck to writeable size and send to client.
                    let cap = res.map_err(DispatchError::SendData)?;

                    let len = chunk.len();
                    let bytes = chunk.split_to(cmp::min(cap, len));

                    stream
                        .send_data(bytes, false)
                        .map_err(DispatchError::SendData)?;

                    // Current chuck completely sent. break send loop and poll next one.
                    if chunk.is_empty() {
                        break 'send;
                    }
                }
            }
        }
    }

    // response body streaming finished. send end of stream and return.
    stream
        .send_data(Bytes::new(), true)
        .map_err(DispatchError::SendData)?;

    Ok(())
}

fn prepare_response(
    config: ServiceConfig,
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

        BodySize::Sized(0) => res
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
        config.set_date_header(&mut bytes);
        res.headers_mut().insert(
            DATE,
            // SAFETY: serialized date-times are known ASCII strings
            unsafe { HeaderValue::from_maybe_shared_unchecked(bytes.freeze()) },
        );
    }

    res
}
