//! Traits and structures to aid consuming and writing HTTP payloads.
//!
//! "Body" and "payload" are used somewhat interchangeably in this documentation.

// Though the spec kinda reads like "payload" is the possibly-transfer-encoded part of the message
// and the "body" is the intended possibly-decoded version of that.

mod body_stream;
mod boxed;
mod either;
mod message_body;
mod none;
mod size;
mod sized_stream;
mod utils;

pub(crate) use self::message_body::MessageBodyMapErr;
pub use self::{
    body_stream::BodyStream,
    boxed::BoxBody,
    either::EitherBody,
    message_body::MessageBody,
    none::None,
    size::BodySize,
    sized_stream::SizedStream,
    utils::{to_bytes, to_bytes_limited, BodyLimitExceeded},
};
