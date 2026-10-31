//! Streaming compression and decompression of HTTP bodies.
//!
//! [`Decoder`] decompresses a payload stream using a given content encoding or the
//! `Content-Encoding` header. [`Encoder`] compresses a response body and updates its response head.
//! The caller is responsible for choosing a response encoding acceptable to the client.
//!
//! # Supported Encodings
//!
//! Supports a subset of the encodings in the [IANA HTTP Content Coding Registry]. Codec support is
//! controlled by crate features:
//!
//! - `compress-brotli`: Brotli (`br`).
//! - `compress-gzip`: Gzip (`gzip`) and zlib-wrapped DEFLATE (`deflate`).
//! - `compress-zstd`: Zstandard (`zstd`).
//!
//! This module is available when at least one of these features is enabled. Identity encoding and
//! encodings whose feature is disabled pass through without compression or decompression.
//!
//! [IANA HTTP Content Coding Registry]: https://www.iana.org/assignments/http-parameters/http-parameters.xhtml#content-coding
//!
//! # Streaming
//!
//! Bodies are processed as they are polled. Output chunk boundaries can differ from input chunk
//! boundaries. Larger chunks are processed on Tokio's blocking thread pool, so poll these adapters
//! within a Tokio runtime.

use std::io;

use bytes::{Bytes, BytesMut};

mod decoder;
mod encoder;

pub use self::{decoder::Decoder, encoder::Encoder};

/// Special-purpose writer for streaming (de-)compression.
///
/// Pre-allocates 8KiB of capacity.
struct Writer {
    buf: BytesMut,
}

impl Writer {
    fn new() -> Writer {
        Writer {
            buf: BytesMut::with_capacity(8192),
        }
    }

    fn take(&mut self) -> Bytes {
        self.buf.split().freeze()
    }
}

impl io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
