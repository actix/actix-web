//! HTTP/1 protocol implementation.

use bytes::{Bytes, BytesMut};

mod chunked;
mod client;
mod codec;
mod decoder;
mod dispatcher;
#[cfg(test)]
mod dispatcher_tests;
mod encoder;
mod expect;
mod payload;
mod service;
mod timer;
mod upgrade;
mod utils;

pub use self::{
    client::{ClientCodec, ClientPayloadCodec},
    codec::Codec,
    dispatcher::Dispatcher,
    expect::ExpectHandler,
    payload::Payload,
    service::{H1Service, H1ServiceHandler},
    upgrade::UpgradeHandler,
    utils::SendResponse,
};

#[derive(Debug)]
/// Codec message
pub enum Message<T> {
    /// HTTP message.
    Item(T),

    /// Payload chunk.
    Chunk(Option<Bytes>),
}

impl<T> From<T> for Message<T> {
    fn from(item: T) -> Self {
        Message::Item(item)
    }
}

/// Incoming request type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    None,
    Payload,
    Stream,
}

const LW: usize = 2 * 1024;
const HW: usize = 32 * 1024;

pub(crate) fn reserve_readbuf(src: &mut BytesMut) {
    let cap = src.capacity();
    if cap < LW {
        src.reserve(HW - cap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Request;

    impl Message<Request> {
        pub fn message(self) -> Request {
            match self {
                Message::Item(req) => req,
                _ => panic!("error"),
            }
        }

        pub fn chunk(self) -> Bytes {
            match self {
                Message::Chunk(Some(data)) => data,
                _ => panic!("error"),
            }
        }

        pub fn eof(self) -> bool {
            match self {
                Message::Chunk(None) => true,
                Message::Chunk(Some(_)) => false,
                _ => panic!("error"),
            }
        }
    }
}
