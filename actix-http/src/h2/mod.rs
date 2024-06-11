//! HTTP/2 protocol.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_rt::time::{sleep_until, Sleep};
use bytes::Bytes;
use futures_core::{ready, Stream};
use h2::{
    server::{handshake, Connection, Handshake},
    RecvStream,
};

use crate::{
    config::ServiceConfig,
    error::{DispatchError, PayloadError},
};

mod dispatcher;
mod service;

pub use self::{dispatcher::Dispatcher, service::H2Service};

/// HTTP/2 peer stream.
pub struct Payload {
    stream: RecvStream,
}

impl Payload {
    pub(crate) fn new(stream: RecvStream) -> Self {
        Self { stream }
    }
}

impl Stream for Payload {
    type Item = Result<Bytes, PayloadError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        match ready!(Pin::new(&mut this.stream).poll_data(cx)) {
            Some(Ok(chunk)) => {
                let len = chunk.len();

                match this.stream.flow_control().release_capacity(len) {
                    Ok(()) => Poll::Ready(Some(Ok(chunk))),
                    Err(err) => Poll::Ready(Some(Err(err.into()))),
                }
            }
            Some(Err(err)) => Poll::Ready(Some(Err(err.into()))),
            None => Poll::Ready(None),
        }
    }
}

pub(crate) fn handshake_with_timeout<T>(io: T, config: &ServiceConfig) -> HandshakeWithTimeout<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    HandshakeWithTimeout {
        handshake: handshake(io),
        timer: config
            .client_request_deadline()
            .map(|deadline| Box::pin(sleep_until(deadline.into()))),
    }
}

pub(crate) struct HandshakeWithTimeout<T: AsyncRead + AsyncWrite + Unpin> {
    handshake: Handshake<T>,
    timer: Option<Pin<Box<Sleep>>>,
}

impl<T> Future for HandshakeWithTimeout<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<(Connection<T, Bytes>, Option<Pin<Box<Sleep>>>), DispatchError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match Pin::new(&mut this.handshake).poll(cx)? {
            // return the timer on success handshake; its slot can be re-used for h2 ping-pong
            Poll::Ready(conn) => Poll::Ready(Ok((conn, this.timer.take()))),
            Poll::Pending => match this.timer.as_mut() {
                Some(timer) => {
                    ready!(timer.as_mut().poll(cx));
                    Poll::Ready(Err(DispatchError::SlowRequestTimeout))
                }
                None => Poll::Pending,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use static_assertions::assert_impl_all;

    use super::*;

    assert_impl_all!(Payload: Unpin, Send, Sync);
}
