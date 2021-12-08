//! Content-Encoding support.

use std::{error::Error as StdError, io};

use bytes::{Bytes, BytesMut};
use derive_more::Display;

use crate::error::BlockingError;

#[cfg(feature = "__compress")]
mod decoder;
#[cfg(feature = "__compress")]
mod encoder;

#[cfg(feature = "__compress")]
pub use self::decoder::Decoder;
#[cfg(feature = "__compress")]
pub use self::encoder::Encoder;

#[derive(Debug, Display)]
#[non_exhaustive]
pub enum EncoderError {
    #[display(fmt = "body")]
    Body(Box<dyn StdError>),

    #[display(fmt = "blocking")]
    Blocking(BlockingError),

    #[display(fmt = "io")]
    Io(io::Error),
}

impl StdError for EncoderError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            EncoderError::Body(err) => Some(&**err),
            EncoderError::Blocking(err) => Some(err),
            EncoderError::Io(err) => Some(err),
        }
    }
}

impl From<EncoderError> for crate::Error {
    fn from(err: EncoderError) -> Self {
        crate::Error::new_encoder().with_cause(err)
    }
}

/// Special-purpose writer for streaming (de-)compression.
///
/// Pre-allocates 8KiB of capacity.
#[cfg(feature = "__compress")]
pub(self) struct Writer {
    buf: BytesMut,
}

#[cfg(feature = "__compress")]
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

#[cfg(feature = "__compress")]
impl io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
