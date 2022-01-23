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

pub use self::body_stream::BodyStream;
pub use self::boxed::BoxBody;
pub use self::either::EitherBody;
pub use self::message_body::MessageBody;
pub(crate) use self::message_body::MessageBodyMapErr;
pub use self::none::None;
pub use self::size::BodySize;
pub use self::sized_stream::SizedStream;
pub use self::utils::to_bytes;
