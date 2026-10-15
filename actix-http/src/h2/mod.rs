//! HTTP/2 protocol.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite};
use bytes::Bytes;
use futures_core::{ready, Stream};
use h2_04::{
    server::{Builder, Connection, Handshake},
    RecvStream,
};
use tokio::time::{sleep_until, Sleep};

use crate::{
    config::ServiceConfig,
    error::{DispatchError, PayloadError},
};

mod compat;
mod dispatcher;
#[cfg(feature = "http2")]
mod legacy;
mod service;

pub use self::{dispatcher::Dispatcher, service::H2Service};

/// HTTP/2 peer stream.
pub struct Payload {
    stream: PayloadStream,
}

enum PayloadStream {
    #[cfg(feature = "http2")]
    Legacy(h2::RecvStream),
    Current(RecvStream),
}

impl Payload {
    #[cfg(feature = "http2")]
    pub(crate) fn new(stream: h2::RecvStream) -> Self {
        Self {
            stream: PayloadStream::Legacy(stream),
        }
    }

    fn from_current(stream: RecvStream) -> Self {
        Self {
            stream: PayloadStream::Current(stream),
        }
    }
}

impl Stream for Payload {
    type Item = Result<Bytes, PayloadError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        let data = match &mut this.stream {
            #[cfg(feature = "http2")]
            PayloadStream::Legacy(stream) => {
                ready!(stream.poll_data(cx)).map(|res| res.map_err(PayloadError::from))
            }
            PayloadStream::Current(stream) => {
                ready!(stream.poll_data(cx)).map(|res| res.map_err(compat::payload_error))
            }
        };

        match data {
            Some(Ok(chunk)) => {
                let len = chunk.len();

                let released = match &mut this.stream {
                    #[cfg(feature = "http2")]
                    PayloadStream::Legacy(stream) => stream
                        .flow_control()
                        .release_capacity(len)
                        .map_err(PayloadError::from),
                    PayloadStream::Current(stream) => stream
                        .flow_control()
                        .release_capacity(len)
                        .map_err(compat::payload_error),
                };

                match released {
                    Ok(()) => Poll::Ready(Some(Ok(chunk))),
                    Err(err) => Poll::Ready(Some(Err(err))),
                }
            }
            Some(Err(err)) => Poll::Ready(Some(Err(err))),
            None => Poll::Ready(None),
        }
    }
}

pub(crate) fn handshake_with_timeout<T>(io: T, config: &ServiceConfig) -> HandshakeWithTimeout<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let mut builder = Builder::new();
    builder
        .initial_window_size(config.h2_initial_window_size())
        .initial_connection_window_size(config.h2_initial_connection_window_size());

    HandshakeWithTimeout {
        handshake: builder.handshake(io),
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

        match Pin::new(&mut this.handshake)
            .poll(cx)
            .map_err(compat::dispatch_error)?
        {
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
    use static_assertions::{assert_impl_all, assert_not_impl_any};

    use super::*;

    assert_impl_all!(Payload: Unpin, Send, Sync);

    // The backend dependency must not become a public conversion interface.
    assert_not_impl_any!(crate::Payload: From<h2_04::RecvStream>);
    assert_not_impl_any!(PayloadError: From<h2_04::Error>);
    assert_not_impl_any!(DispatchError: From<h2_04::Error>);
    assert_not_impl_any!(crate::error::Http2Error: From<h2_04::Error>);
}
