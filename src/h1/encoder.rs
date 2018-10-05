#![allow(unused_imports, unused_variables, dead_code)]
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::str::FromStr;
use std::{cmp, fmt, io, mem};

use bytes::{Bytes, BytesMut};
use http::header::{HeaderValue, ACCEPT_ENCODING, CONTENT_LENGTH};
use http::{StatusCode, Version};

use body::{Binary, Body};
use header::ContentEncoding;
use http::Method;
use request::Request;
use response::Response;

#[derive(Debug)]
pub(crate) enum ResponseLength {
    Chunked,
    Zero,
    Length(usize),
    Length64(u64),
    None,
}

#[derive(Debug)]
pub(crate) struct ResponseEncoder {
    head: bool,
    pub length: ResponseLength,
    pub te: TransferEncoding,
}

impl Default for ResponseEncoder {
    fn default() -> Self {
        ResponseEncoder {
            head: false,
            length: ResponseLength::None,
            te: TransferEncoding::empty(),
        }
    }
}

impl ResponseEncoder {
    pub fn update(&mut self, resp: &mut Response, head: bool, version: Version) {
        self.head = head;

        let version = resp.version().unwrap_or_else(|| version);
        let mut len = 0;

        let has_body = match resp.body() {
            Body::Empty => false,
            Body::Binary(ref bin) => {
                len = bin.len();
                true
            }
            _ => true,
        };

        let has_body = match resp.body() {
            Body::Empty => false,
            _ => true,
        };

        let transfer = match resp.body() {
            Body::Empty => {
                if !self.head {
                    self.length = match resp.status() {
                        StatusCode::NO_CONTENT
                        | StatusCode::CONTINUE
                        | StatusCode::SWITCHING_PROTOCOLS
                        | StatusCode::PROCESSING => ResponseLength::None,
                        _ => ResponseLength::Zero,
                    };
                } else {
                    self.length = ResponseLength::Zero;
                }
                TransferEncoding::empty()
            }
            Body::Binary(_) => {
                self.length = ResponseLength::Length(len);
                TransferEncoding::length(len as u64)
            }
            Body::Streaming(_) => {
                if resp.upgrade() {
                    self.length = ResponseLength::None;
                    TransferEncoding::eof()
                } else {
                    self.streaming_encoding(version, resp)
                }
            }
        };
        // check for head response
        if self.head {
            resp.set_body(Body::Empty);
        } else {
            self.te = transfer;
        }
    }

    fn streaming_encoding(
        &mut self, version: Version, resp: &mut Response,
    ) -> TransferEncoding {
        match resp.chunked() {
            Some(true) => {
                // Enable transfer encoding
                if version == Version::HTTP_2 {
                    self.length = ResponseLength::None;
                    TransferEncoding::eof()
                } else {
                    self.length = ResponseLength::Chunked;
                    TransferEncoding::chunked()
                }
            }
            Some(false) => TransferEncoding::eof(),
            None => {
                // if Content-Length is specified, then use it as length hint
                let (len, chunked) =
                    if let Some(len) = resp.headers().get(CONTENT_LENGTH) {
                        // Content-Length
                        if let Ok(s) = len.to_str() {
                            if let Ok(len) = s.parse::<u64>() {
                                (Some(len), false)
                            } else {
                                error!("illegal Content-Length: {:?}", len);
                                (None, false)
                            }
                        } else {
                            error!("illegal Content-Length: {:?}", len);
                            (None, false)
                        }
                    } else {
                        (None, true)
                    };

                if !chunked {
                    if let Some(len) = len {
                        self.length = ResponseLength::Length64(len);
                        TransferEncoding::length(len)
                    } else {
                        TransferEncoding::eof()
                    }
                } else {
                    // Enable transfer encoding
                    match version {
                        Version::HTTP_11 => {
                            self.length = ResponseLength::Chunked;
                            TransferEncoding::chunked()
                        }
                        _ => {
                            self.length = ResponseLength::None;
                            TransferEncoding::eof()
                        }
                    }
                }
            }
        }
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
    pub fn encode_eof(&mut self, buf: &mut BytesMut) -> bool {
        match self.kind {
            TransferEncodingKind::Eof => true,
            TransferEncodingKind::Length(rem) => rem == 0,
            TransferEncodingKind::Chunked(ref mut eof) => {
                if !*eof {
                    *eof = true;
                    buf.extend_from_slice(b"0\r\n\r\n");
                }
                true
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
