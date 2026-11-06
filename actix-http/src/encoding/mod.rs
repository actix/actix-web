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

/// Special-purpose writer for streaming (de-)compression and tracking size limits.
///
/// Pre-allocates 8KiB of capacity.
struct Writer {
    buf: BytesMut,

    /// When limit set, number of allowed bytes remaining.
    remaining: Option<usize>,
}

impl Writer {
    fn new() -> Writer {
        Writer {
            buf: BytesMut::with_capacity(8192),
            remaining: None,
        }
    }

    fn set_limit(&mut self, limit: usize) {
        self.remaining = Some(limit);
    }

    fn take(&mut self) -> Bytes {
        self.buf.split().freeze()
    }
}

impl io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(remaining) = &mut self.remaining {
            if buf.len() > *remaining {
                *remaining = 0;
                return Err(io::ErrorKind::FileTooLarge.into());
            }

            *remaining -= buf.len();
        }

        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::Writer;

    #[test]
    fn writer_rejects_overflow_without_copying() {
        let mut writer = Writer::new();
        writer.set_limit(3);
        writer.write_all(b"ab").unwrap();
        assert!(writer.write_all(b"cdef").is_err());

        assert_eq!(writer.take().as_ref(), b"ab");

        assert!(writer.write_all(b"more output").is_err());
        assert!(writer.take().is_empty());
    }

    #[test]
    fn writer_zero_limit_rejects_output() {
        let mut writer = Writer::new();
        writer.set_limit(0);
        assert!(writer.write_all(b"ab").is_err());

        assert!(writer.take().is_empty());
    }

    #[test]
    fn taking_output_does_not_reset_limit() {
        let mut writer = Writer::new();
        writer.set_limit(3);
        writer.write_all(b"ab").unwrap();
        assert_eq!(writer.take().as_ref(), b"ab");

        writer.write_all(b"c").unwrap();
        assert!(writer.write_all(b"d").is_err());
        assert_eq!(writer.take().as_ref(), b"c");
    }

    #[test]
    fn writer_max_limit_does_not_wrap() {
        let mut writer = Writer::new();
        writer.set_limit(usize::MAX);
        writer.write_all(b"abc").unwrap();

        assert_eq!(writer.take().as_ref(), b"abc");
    }

    #[test]
    fn writer_is_unlimited_by_default() {
        let mut writer = Writer::new();
        let data = vec![0; 1024 * 1024];
        writer.write_all(&data).unwrap();

        assert_eq!(writer.take().as_ref(), data);
    }
}
