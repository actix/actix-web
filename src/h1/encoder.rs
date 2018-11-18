#![allow(unused_imports, unused_variables, dead_code)]
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::str::FromStr;
use std::{cmp, fmt, io, mem};

use bytes::{Bytes, BytesMut};
use http::header::{HeaderValue, ACCEPT_ENCODING, CONTENT_LENGTH};
use http::{StatusCode, Version};

use body::{Binary, BodyLength};
use header::ContentEncoding;
use http::Method;
use message::{RequestHead, ResponseHead};
use request::Request;
use response::Response;

#[derive(Debug)]
pub(crate) struct ResponseEncoder {
    head: bool,
    pub length: BodyLength,
    pub te: TransferEncoding,
}

impl Default for ResponseEncoder {
    fn default() -> Self {
        ResponseEncoder {
            head: false,
            length: BodyLength::None,
            te: TransferEncoding::empty(),
        }
    }
}

impl ResponseEncoder {
    /// Encode message
    pub fn encode(&mut self, msg: &[u8], buf: &mut BytesMut) -> io::Result<bool> {
        self.te.encode(msg, buf)
    }

    /// Encode eof
    pub fn encode_eof(&mut self, buf: &mut BytesMut) -> io::Result<()> {
        self.te.encode_eof(buf)
    }

    pub fn update(
        &mut self,
        resp: &mut ResponseHead,
        head: bool,
        version: Version,
        length: &mut BodyLength,
    ) {
        self.head = head;
        let transfer = match length {
            BodyLength::Zero => {
                match resp.status {
                    StatusCode::NO_CONTENT
                    | StatusCode::CONTINUE
                    | StatusCode::SWITCHING_PROTOCOLS
                    | StatusCode::PROCESSING => *length = BodyLength::None,
                    _ => (),
                }
                TransferEncoding::empty()
            }
            BodyLength::Sized(len) => TransferEncoding::length(*len as u64),
            BodyLength::Sized64(len) => TransferEncoding::length(*len),
            BodyLength::Chunked => TransferEncoding::chunked(),
            BodyLength::Stream => TransferEncoding::eof(),
            BodyLength::None => TransferEncoding::length(0),
        };
        // check for head response
        if !self.head {
            self.te = transfer;
        }
    }
}

#[derive(Debug)]
pub(crate) struct RequestEncoder {
    head: bool,
    pub length: BodyLength,
    pub te: TransferEncoding,
}

impl Default for RequestEncoder {
    fn default() -> Self {
        RequestEncoder {
            head: false,
            length: BodyLength::None,
            te: TransferEncoding::empty(),
        }
    }
}

impl RequestEncoder {
    /// Encode message
    pub fn encode(&mut self, msg: &[u8], buf: &mut BytesMut) -> io::Result<bool> {
        self.te.encode(msg, buf)
    }

    /// Encode eof
    pub fn encode_eof(&mut self, buf: &mut BytesMut) -> io::Result<()> {
        self.te.encode_eof(buf)
    }

    pub fn update(&mut self, resp: &mut RequestHead, head: bool, version: Version) {
        self.head = head;
    }
}

/// Encoders to handle different Transfer-Encodings.
#[derive(Debug)]
pub(crate) struct TransferEncoding {
    kind: TransferEncodingKind,
}

#[derive(Debug, PartialEq, Clone)]
enum TransferEncodingKind {
    /// An Encoder for when Transfer-Encoding includes `chunked`.
    Chunked(bool),
    /// An Encoder for when Content-Length is set.
    ///
    /// Enforces that the body is not longer than the Content-Length header.
    Length(u64),
    /// An Encoder for when Content-Length is not known.
    ///
    /// Application decides when to stop writing.
    Eof,
}

impl TransferEncoding {
    #[inline]
    pub fn empty() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Eof,
        }
    }

    #[inline]
    pub fn eof() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Eof,
        }
    }

    #[inline]
    pub fn chunked() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Chunked(false),
        }
    }

    #[inline]
    pub fn length(len: u64) -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Length(len),
        }
    }

    /// Encode message. Return `EOF` state of encoder
    #[inline]
    pub fn encode(&mut self, msg: &[u8], buf: &mut BytesMut) -> io::Result<bool> {
        match self.kind {
            TransferEncodingKind::Eof => {
                let eof = msg.is_empty();
                buf.extend_from_slice(msg);
                Ok(eof)
            }
            TransferEncodingKind::Chunked(ref mut eof) => {
                if *eof {
                    return Ok(true);
                }

                if msg.is_empty() {
                    *eof = true;
                    buf.extend_from_slice(b"0\r\n\r\n");
                } else {
                    writeln!(Writer(buf), "{:X}\r", msg.len())
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

                    buf.reserve(msg.len() + 2);
                    buf.extend_from_slice(msg);
                    buf.extend_from_slice(b"\r\n");
                }
                Ok(*eof)
            }
            TransferEncodingKind::Length(ref mut remaining) => {
                if *remaining > 0 {
                    if msg.is_empty() {
                        return Ok(*remaining == 0);
                    }
                    let len = cmp::min(*remaining, msg.len() as u64);

                    buf.extend_from_slice(&msg[..len as usize]);

                    *remaining -= len as u64;
                    Ok(*remaining == 0)
                } else {
                    Ok(true)
                }
            }
        }
    }

    /// Encode eof. Return `EOF` state of encoder
    #[inline]
    pub fn encode_eof(&mut self, buf: &mut BytesMut) -> io::Result<()> {
        match self.kind {
            TransferEncodingKind::Eof => Ok(()),
            TransferEncodingKind::Length(rem) => {
                if rem != 0 {
                    Err(io::Error::new(io::ErrorKind::UnexpectedEof, ""))
                } else {
                    Ok(())
                }
            }
            TransferEncodingKind::Chunked(ref mut eof) => {
                if !*eof {
                    *eof = true;
                    buf.extend_from_slice(b"0\r\n\r\n");
                }
                Ok(())
            }
        }
    }
}

struct Writer<'a>(pub &'a mut BytesMut);

impl<'a> io::Write for Writer<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_chunked_te() {
        let mut bytes = BytesMut::new();
        let mut enc = TransferEncoding::chunked();
        {
            assert!(!enc.encode(b"test", &mut bytes).ok().unwrap());
            assert!(enc.encode(b"", &mut bytes).ok().unwrap());
        }
        assert_eq!(
            bytes.take().freeze(),
            Bytes::from_static(b"4\r\ntest\r\n0\r\n\r\n")
        );
    }
}
