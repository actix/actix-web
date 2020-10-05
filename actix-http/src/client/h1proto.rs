use std::io::Write;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{io, mem, time};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use bytes::buf::BufMutExt;
use bytes::{Bytes, BytesMut};
use futures_core::Stream;
use futures_util::future::poll_fn;
use futures_util::{pin_mut, SinkExt, StreamExt};

use crate::error::PayloadError;
use crate::h1;
use crate::header::HeaderMap;
use crate::http::header::{IntoHeaderValue, HOST};
use crate::message::{RequestHeadType, ResponseHead};
use crate::payload::{Payload, PayloadStream};

use super::connection::{ConnectionLifetime, ConnectionType, IoConnection};
use super::error::{ConnectError, SendRequestError};
use super::pool::Acquired;
use crate::body::{BodySize, MessageBody};

pub(crate) async fn send_request<T, B>(
    io: T,
    mut head: RequestHeadType,
    body: B,
    created: time::Instant,
    pool: Option<Acquired<T>>,
) -> Result<(ResponseHead, Payload), SendRequestError>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
    B: MessageBody,
{
    // set request host header
    if !head.as_ref().headers.contains_key(HOST)
        && !head.extra_headers().iter().any(|h| h.contains_key(HOST))
    {
        if let Some(host) = head.as_ref().uri.host() {
            let mut wrt = BytesMut::with_capacity(host.len() + 5).writer();

            let _ = match head.as_ref().uri.port_u16() {
                None | Some(80) | Some(443) => write!(wrt, "{}", host),
                Some(port) => write!(wrt, "{}:{}", host, port),
            };

            match wrt.get_mut().split().freeze().try_into() {
                Ok(value) => match head {
                    RequestHeadType::Owned(ref mut head) => {
                        head.headers.insert(HOST, value)
                    }
                    RequestHeadType::Rc(_, ref mut extra_headers) => {
                        let headers = extra_headers.get_or_insert(HeaderMap::new());
                        headers.insert(HOST, value)
                    }
                },
                Err(e) => log::error!("Can not set HOST header {}", e),
            }
        }
    }

    let io = H1Connection {
        created,
        pool,
        io: Some(io),
    };

    // create Framed and send request
    let mut framed_inner = Framed::new(io, h1::ClientCodec::default());
    framed_inner.send((head, body.size()).into()).await?;

    // send request body
    match body.size() {
        BodySize::None | BodySize::Empty | BodySize::Sized(0) => (),
        _ => send_body(body, Pin::new(&mut framed_inner)).await?,
    };

    // read response and init read body
    let res = Pin::new(&mut framed_inner).into_future().await;
    let (head, framed) = if let (Some(result), framed) = res {
        let item = result.map_err(SendRequestError::from)?;
        (item, framed)
    } else {
        return Err(SendRequestError::from(ConnectError::Disconnected));
    };

    match framed.codec_ref().message_type() {
        h1::MessageType::None => {
            let force_close = !framed.codec_ref().keepalive();
            release_connection(framed, force_close);
            Ok((head, Payload::None))
        }
        _ => {
            let pl: PayloadStream = PlStream::new(framed_inner).boxed_local();
            Ok((head, pl.into()))
        }
    }
}

pub(crate) async fn open_tunnel<T>(
    io: T,
    head: RequestHeadType,
) -> Result<(ResponseHead, Framed<T, h1::ClientCodec>), SendRequestError>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    // create Framed and send request
    let mut framed = Framed::new(io, h1::ClientCodec::default());
    framed.send((head, BodySize::None).into()).await?;

    // read response
    if let (Some(result), framed) = framed.into_future().await {
        let head = result.map_err(SendRequestError::from)?;
        Ok((head, framed))
    } else {
        Err(SendRequestError::from(ConnectError::Disconnected))
    }
}

/// send request body to the peer
pub(crate) async fn send_body<T, B>(
    body: B,
    mut framed: Pin<&mut Framed<T, h1::ClientCodec>>,
) -> Result<(), SendRequestError>
where
    T: ConnectionLifetime + Unpin,
    B: MessageBody,
{
    pin_mut!(body);

    let mut eof = false;
    while !eof {
        while !eof && !framed.as_ref().is_write_buf_full() {
            match poll_fn(|cx| body.as_mut().poll_next(cx)).await {
                Some(result) => {
                    framed.as_mut().write(h1::Message::Chunk(Some(result?)))?;
                }
                None => {
                    eof = true;
                    framed.as_mut().write(h1::Message::Chunk(None))?;
                }
            }
        }

        if !framed.as_ref().is_write_buf_empty() {
            poll_fn(|cx| match framed.as_mut().flush(cx) {
                Poll::Ready(Ok(_)) => Poll::Ready(Ok(())),
                Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
                Poll::Pending => {
                    if !framed.as_ref().is_write_buf_full() {
                        Poll::Ready(Ok(()))
                    } else {
                        Poll::Pending
                    }
                }
            })
            .await?;
        }
    }

    SinkExt::flush(Pin::into_inner(framed)).await?;
    Ok(())
}

#[doc(hidden)]
/// HTTP client connection
pub struct H1Connection<T> {
    /// T should be `Unpin`
    io: Option<T>,
    created: time::Instant,
    pool: Option<Acquired<T>>,
}

impl<T> ConnectionLifetime for H1Connection<T>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    /// Close connection
    fn close(mut self: Pin<&mut Self>) {
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
    fn release(mut self: Pin<&mut Self>) {
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

impl<T: AsyncRead + AsyncWrite + Unpin + 'static> AsyncRead for H1Connection<T> {
    unsafe fn prepare_uninitialized_buffer(
        &self,
        buf: &mut [mem::MaybeUninit<u8>],
    ) -> bool {
        self.io.as_ref().unwrap().prepare_uninitialized_buffer(buf)
    }

    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io.as_mut().unwrap()).poll_read(cx, buf)
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + 'static> AsyncWrite for H1Connection<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io.as_mut().unwrap()).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(self.io.as_mut().unwrap()).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(self.io.as_mut().unwrap()).poll_shutdown(cx)
    }
}

#[pin_project::pin_project]
pub(crate) struct PlStream<Io> {
    #[pin]
    framed: Option<Framed<Io, h1::ClientPayloadCodec>>,
}

impl<Io: ConnectionLifetime> PlStream<Io> {
    fn new(framed: Framed<Io, h1::ClientCodec>) -> Self {
        let framed = framed.into_map_codec(|codec| codec.into_payload_codec());

        PlStream {
            framed: Some(framed),
        }
    }
}

impl<Io: ConnectionLifetime> Stream for PlStream<Io> {
    type Item = Result<Bytes, PayloadError>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        match this.framed.as_mut().as_pin_mut().unwrap().next_item(cx)? {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(chunk)) => {
                if let Some(chunk) = chunk {
                    Poll::Ready(Some(Ok(chunk)))
                } else {
                    let framed = this.framed.as_mut().as_pin_mut().unwrap();
                    let force_close = !framed.codec_ref().keepalive();
                    release_connection(framed, force_close);
                    Poll::Ready(None)
                }
            }
            Poll::Ready(None) => Poll::Ready(None),
        }
    }
}

fn release_connection<T, U>(framed: Pin<&mut Framed<T, U>>, force_close: bool)
where
    T: ConnectionLifetime,
{
    if !force_close && framed.is_read_buf_empty() && framed.is_write_buf_empty() {
        framed.io_pin().release()
    } else {
        framed.io_pin().close()
    }
}
