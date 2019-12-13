//! HTTP/2 implementation
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_core::Stream;
use h2::RecvStream;

mod dispatcher;
mod service;

pub use self::dispatcher::Dispatcher;
pub use self::service::H2Service;
use crate::error::PayloadError;

/// H2 receive stream
pub struct Payload {
    pl: RecvStream,
}

impl Payload {
    pub(crate) fn new(pl: RecvStream) -> Self {
        Self { pl }
    }
}

impl Stream for Payload {
    type Item = Result<Bytes, PayloadError>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        match Pin::new(&mut this.pl).poll_data(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                let len = chunk.len();
                if let Err(err) = this.pl.flow_control().release_capacity(len) {
                    Poll::Ready(Some(Err(err.into())))
                } else {
                    Poll::Ready(Some(Ok(chunk)))
                }
            }
            Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(err.into()))),
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
        }
    }
}
