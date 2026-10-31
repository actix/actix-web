//! HTTP/2 protocol.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite};
use bytes::Bytes;
use futures_core::{ready, Stream};
use h2::{
    server::{Builder, Connection, Handshake},
    RecvStream,
};
use tokio::time::{sleep_until, Sleep};

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
    let mut builder = Builder::new();
    builder
        .initial_window_size(config.h2_initial_window_size())
        .initial_connection_window_size(config.h2_initial_connection_window_size());

    if let Some(size) = config.h2_max_header_list_size() {
        builder.max_header_list_size(size);
    }

    if let Some(max) = config.h2_max_concurrent_streams() {
        builder.max_concurrent_streams(max);
    }

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
    use tokio::net::TcpStream;

    use super::*;

    assert_impl_all!(Payload: Unpin, Send, Sync);

    async fn client_settings(
        config: ServiceConfig,
    ) -> (
        h2::client::SendRequest<Bytes>,
        h2::client::Connection<TcpStream, Bytes>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        actix_rt::spawn(async move {
            let (io, _) = listener.accept().await.unwrap();
            let (mut conn, _timer) = handshake_with_timeout(io, &config).await.unwrap();
            while let Some(Ok((_req, mut res))) = conn.accept().await {
                let _ = res.send_response(http::Response::new(()), true);
            }
        });

        let io = TcpStream::connect(addr).await.unwrap();
        let (send, mut conn) = h2::client::handshake(io).await.unwrap();

        // drive the connection until the server's first SETTINGS frame has been applied
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while conn.max_concurrent_send_streams() == usize::MAX
            && tokio::time::Instant::now() < deadline
        {
            tokio::select! {
                _ = &mut conn => break,
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
            }
        }

        (send, conn)
    }

    #[actix_rt::test]
    async fn max_concurrent_streams_is_advertised() {
        let config = crate::ServiceConfigBuilder::new()
            .h2_max_concurrent_streams(Some(100))
            .build();

        let (_send, conn) = client_settings(config).await;
        assert_eq!(conn.max_concurrent_send_streams(), 100);
    }

    #[actix_rt::test]
    async fn max_header_list_size_is_enforced() {
        let config = crate::ServiceConfigBuilder::new()
            .h2_max_header_list_size(Some(16 * 1024))
            .build();

        let (send, conn) = client_settings(config).await;
        actix_rt::spawn(async move {
            let _ = conn.await;
        });
        let mut send = send.ready().await.unwrap();

        let small = http::Request::get("http://localhost/").body(()).unwrap();
        let (res, _) = send.send_request(small, true).unwrap();
        assert_eq!(res.await.unwrap().status(), http::StatusCode::OK);

        let mut big = http::Request::get("http://localhost/").body(()).unwrap();
        for i in 0..20 {
            big.headers_mut().insert(
                http::HeaderName::try_from(format!("x-big-{i}")).unwrap(),
                http::HeaderValue::from_str(&"a".repeat(1024)).unwrap(),
            );
        }
        let mut send = send.ready().await.unwrap();
        let refused = match send.send_request(big, true) {
            Err(_) => true,
            Ok((res, _)) => match res.await {
                Err(_) => true,
                Ok(res) => res.status() != http::StatusCode::OK,
            },
        };
        assert!(refused, "a 20 KiB header list passed a 16 KiB limit");
    }

    #[actix_rt::test]
    async fn h2_limits_are_unset_by_default() {
        let config = ServiceConfig::default();
        assert_eq!(config.h2_max_header_list_size(), None);
        assert_eq!(config.h2_max_concurrent_streams(), None);

        let config = crate::ServiceConfigBuilder::new()
            .h2_max_header_list_size(Some(128 * 1024))
            .build();
        assert_eq!(config.h2_max_header_list_size(), Some(128 * 1024));
    }
}
