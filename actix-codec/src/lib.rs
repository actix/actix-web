//! Utilities for encoding and decoding frames.
//!
//! Contains adapters to go from streams of bytes, [`AsyncRead`] and
//! [`AsyncWrite`], to framed streams implementing [`Sink`] and [`Stream`].
//! Framed streams are also known as [transports].
//!
//! [`AsyncRead`]: #
//! [`AsyncWrite`]: #
//! [`Sink`]: #
//! [`Stream`]: #
//! [transports]: #

// #![deny(missing_docs, missing_debug_implementations, warnings)]

mod bcodec;
mod framed;
// mod framed2;
mod framed_read;
mod framed_write;

pub use self::bcodec::BytesCodec;
pub use self::framed::{Framed, FramedParts};
// pub use self::framed2::{Framed2, FramedParts2};
pub use self::framed_read::FramedRead;
pub use self::framed_write::FramedWrite;

pub use tokio_codec::{Decoder, Encoder};
pub use tokio_io::{AsyncRead, AsyncWrite};
