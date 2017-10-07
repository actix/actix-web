#![allow(dead_code)]

use std::{io, usize};

use futures::{Async, Poll};
use bytes::{Bytes, BytesMut};

use self::Kind::{Length, Chunked, Eof};

/// Decoders to handle different Transfer-Encodings.
///
/// If a message body does not include a Transfer-Encoding, it *should*
/// include a Content-Length header.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoder {
    kind: Kind,
}

impl Decoder {
    pub fn length(x: u64) -> Decoder {
        Decoder { kind: Kind::Length(x) }
    }

    pub fn chunked() -> Decoder {
        Decoder { kind: Kind::Chunked(ChunkedState::Size, 0) }
    }

    pub fn eof() -> Decoder {
        Decoder { kind: Kind::Eof(false) }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// A Reader used when a Content-Length header is passed with a positive integer.
    Length(u64),
    /// A Reader used when Transfer-Encoding is `chunked`.
    Chunked(ChunkedState, u64),
    /// A Reader used for responses that don't indicate a length or chunked.
    ///
    /// Note: This should only used for `Response`s. It is illegal for a
    /// `Request` to be made with both `Content-Length` and
    /// `Transfer-Encoding: chunked` missing, as explained from the spec:
    ///
    /// > If a Transfer-Encoding header field is present in a response and
    /// > the chunked transfer coding is not the final encoding, the
    /// > message body length is determined by reading the connection until
    /// > it is closed by the server.  If a Transfer-Encoding header field
    /// > is present in a request and the chunked transfer coding is not
    /// > the final encoding, the message body length cannot be determined
    /// > reliably; the server MUST respond with the 400 (Bad Request)
    /// > status code and then close the connection.
    Eof(bool),
}

#[derive(Debug, PartialEq, Clone)]
enum ChunkedState {
    Size,
    SizeLws,
    Extension,
    SizeLf,
    Body,
    BodyCr,
    BodyLf,
    EndCr,
    EndLf,
    End,
}

impl Decoder {
    pub fn is_eof(&self) -> bool {
        trace!("is_eof? {:?}", self);
        match self.kind {
            Length(0) |
            Chunked(ChunkedState::End, _) |
            Eof(true) => true,
            _ => false,
        }
    }
}

impl Decoder {
    pub fn decode(&mut self, body: &mut BytesMut) -> Poll<Option<Bytes>, io::Error> {
        match self.kind {
            Length(ref mut remaining) => {
                trace!("Sized read, remaining={:?}", remaining);
                if *remaining == 0 {
                    Ok(Async::Ready(None))
                } else {
                    let len = body.len() as u64;
                    let buf;
                    if *remaining > len {
                        buf = body.take().freeze();
                        *remaining -= len;
                    } else {
                        buf = body.split_to(*remaining as usize).freeze();
                        *remaining = 0;
                    }
                    trace!("Length read: {}", buf.len());
                    Ok(Async::Ready(Some(buf)))
                }
            }
            Chunked(ref mut state, ref mut size) => {
                loop {
                    let mut buf = None;
                    // advances the chunked state
                    *state = try_ready!(state.step(body, size, &mut buf));
                    if *state == ChunkedState::End {
                        trace!("end of chunked");
                        return Ok(Async::Ready(None));
                    }
                    if let Some(buf) = buf {
                        return Ok(Async::Ready(Some(buf)));
                    }
                    if body.is_empty() {
                        return Ok(Async::NotReady);
                    }
                }
            }
            Eof(ref mut is_eof) => {
                if *is_eof {
                    Ok(Async::Ready(None))
                } else if !body.is_empty() {
                    Ok(Async::Ready(Some(body.take().freeze())))
                } else {
                    Ok(Async::NotReady)
                }
            }
        }
    }
}

macro_rules! byte (
    ($rdr:ident) => ({
        if $rdr.len() > 0 {
            let b = $rdr[1];
            $rdr.split_to(1);
            b
        } else {
            return Ok(Async::NotReady)
        }
    })
);

impl ChunkedState {
    fn step(&self, body: &mut BytesMut, size: &mut u64, buf: &mut Option<Bytes>)
            -> Poll<ChunkedState, io::Error>
    {
        use self::ChunkedState::*;
        match *self {
            Size => ChunkedState::read_size(body, size),
            SizeLws => ChunkedState::read_size_lws(body),
            Extension => ChunkedState::read_extension(body),
            SizeLf => ChunkedState::read_size_lf(body, size),
            Body => ChunkedState::read_body(body, size, buf),
            BodyCr => ChunkedState::read_body_cr(body),
            BodyLf => ChunkedState::read_body_lf(body),
            EndCr => ChunkedState::read_end_cr(body),
            EndLf => ChunkedState::read_end_lf(body),
            End => Ok(Async::Ready(ChunkedState::End)),
        }
    }
    fn read_size(rdr: &mut BytesMut, size: &mut u64) -> Poll<ChunkedState, io::Error> {
        trace!("Read chunk hex size");
        let radix = 16;
        match byte!(rdr) {
            b @ b'0'...b'9' => {
                *size *= radix;
                *size += u64::from(b - b'0');
            }
            b @ b'a'...b'f' => {
                *size *= radix;
                *size += u64::from(b + 10 - b'a');
            }
            b @ b'A'...b'F' => {
                *size *= radix;
                *size += u64::from(b + 10 - b'A');
            }
            b'\t' | b' ' => return Ok(Async::Ready(ChunkedState::SizeLws)),
            b';' => return Ok(Async::Ready(ChunkedState::Extension)),
            b'\r' => return Ok(Async::Ready(ChunkedState::SizeLf)),
            _ => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput,
                                          "Invalid chunk size line: Invalid Size"));
            }
        }
        Ok(Async::Ready(ChunkedState::Size))
    }
    fn read_size_lws(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        trace!("read_size_lws");
        match byte!(rdr) {
            // LWS can follow the chunk size, but no more digits can come
            b'\t' | b' ' => Ok(Async::Ready(ChunkedState::SizeLws)),
            b';' => Ok(Async::Ready(ChunkedState::Extension)),
            b'\r' => Ok(Async::Ready(ChunkedState::SizeLf)),
            _ => {
                Err(io::Error::new(io::ErrorKind::InvalidInput,
                                   "Invalid chunk size linear white space"))
            }
        }
    }
    fn read_extension(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        trace!("read_extension");
        match byte!(rdr) {
            b'\r' => Ok(Async::Ready(ChunkedState::SizeLf)),
            _ => Ok(Async::Ready(ChunkedState::Extension)), // no supported extensions
        }
    }
    fn read_size_lf(rdr: &mut BytesMut, size: &mut u64) -> Poll<ChunkedState, io::Error> {
        trace!("Chunk size is {:?}", size);
        match byte!(rdr) {
            b'\n' if *size > 0 => Ok(Async::Ready(ChunkedState::Body)),
            b'\n' if *size == 0 => Ok(Async::Ready(ChunkedState::EndCr)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk size LF")),
        }
    }

    fn read_body(rdr: &mut BytesMut, rem: &mut u64, buf: &mut Option<Bytes>)
                 -> Poll<ChunkedState, io::Error>
    {
        trace!("Chunked read, remaining={:?}", rem);

        let len = rdr.len() as u64;
        if len == 0 {
            Ok(Async::Ready(ChunkedState::Body))
        } else {
            let slice;
            if *rem > len {
                slice = rdr.take().freeze();
                *rem -= len;
            } else {
                slice = rdr.split_to(*rem as usize).freeze();
                *rem = 0;
            }
            *buf = Some(slice);
            if *rem > 0 {
                Ok(Async::Ready(ChunkedState::Body))
            } else {
                Ok(Async::Ready(ChunkedState::BodyCr))
            }
        }
    }

    fn read_body_cr(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\r' => Ok(Async::Ready(ChunkedState::BodyLf)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk body CR")),
        }
    }
    fn read_body_lf(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\n' => Ok(Async::Ready(ChunkedState::Size)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk body LF")),
        }
    }
    fn read_end_cr(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\r' => Ok(Async::Ready(ChunkedState::EndLf)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk end CR")),
        }
    }
    fn read_end_lf(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\n' => Ok(Async::Ready(ChunkedState::End)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid chunk end LF")),
        }
    }
}
