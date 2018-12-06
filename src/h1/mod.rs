//! HTTP/1 implementation
use std::fmt;

use actix_net::codec::Framed;
use bytes::Bytes;

mod client;
mod codec;
mod decoder;
mod dispatcher;
mod encoder;
mod service;

pub use self::client::{ClientCodec, ClientPayloadCodec};
pub use self::codec::Codec;
pub use self::dispatcher::Dispatcher;
pub use self::service::{H1Service, H1ServiceHandler, OneRequest};

use crate::request::Request;

/// H1 service response type
pub enum H1ServiceResult<T> {
    Disconnected,
    Shutdown(T),
    Unhandled(Request, Framed<T, Codec>),
}

impl<T: fmt::Debug> fmt::Debug for H1ServiceResult<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            H1ServiceResult::Disconnected => write!(f, "H1ServiceResult::Disconnected"),
            H1ServiceResult::Shutdown(ref v) => {
                write!(f, "H1ServiceResult::Shutdown({:?})", v)
            }
            H1ServiceResult::Unhandled(ref req, _) => {
                write!(f, "H1ServiceResult::Unhandled({:?})", req)
            }
        }
    }
}

#[derive(Debug)]
/// Codec message
pub enum Message<T> {
    /// Http message
    Item(T),
    /// Payload chunk
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

#[cfg(test)]
mod tests {
    use super::*;

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
