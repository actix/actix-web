//! Content-Encoding support.

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
