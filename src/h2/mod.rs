#![allow(dead_code, unused_imports)]

use std::fmt;

use bytes::Bytes;
use futures::{Async, Poll, Stream};
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
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.pl.poll() {
            Ok(Async::Ready(Some(chunk))) => {
                let len = chunk.len();
                if let Err(err) = self.pl.release_capacity().release_capacity(len) {
                    Err(err.into())
                } else {
                    Ok(Async::Ready(Some(chunk)))
                }
            }
            Ok(Async::Ready(None)) => Ok(Async::Ready(None)),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => Err(err.into()),
        }
    }
}
