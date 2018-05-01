use std::{io, mem};

use bytes::{Bytes, BytesMut};
use futures::{Async, Poll};
use httparse;

use super::helpers::SharedHttpInnerMessage;
use super::settings::WorkerSettings;
use error::ParseError;
use http::header::{HeaderName, HeaderValue};
use http::{header, HttpTryFrom, Method, Uri, Version};
use httprequest::MessageFlags;
use uri::Url;

const MAX_BUFFER_SIZE: usize = 131_072;
const MAX_HEADERS: usize = 96;

pub(crate) struct H1Decoder {
    decoder: Option<EncodingDecoder>,
}

pub(crate) enum Message {
    Message {
        msg: SharedHttpInnerMessage,
        payload: bool,
    },
    Chunk(Bytes),
    Eof,
}

#[derive(Debug)]
pub(crate) enum DecoderError {
    Io(io::Error),
    Error(ParseError),
}

impl From<io::Error> for DecoderError {
    fn from(err: io::Error) -> DecoderError {
        DecoderError::Io(err)
    }
}

impl H1Decoder {
    pub fn new() -> H1Decoder {
        H1Decoder { decoder: None }
    }

    pub fn decode<H>(
        &mut self, src: &mut BytesMut, settings: &WorkerSettings<H>,
    ) -> Result<Option<Message>, DecoderError> {
        // read payload
        if self.decoder.is_some() {
            match self.decoder.as_mut().unwrap().decode(src)? {
                Async::Ready(Some(bytes)) => return Ok(Some(Message::Chunk(bytes))),
                Async::Ready(None) => {
                    self.decoder.take();
                    return Ok(Some(Message::Eof));
                }
                Async::NotReady => return Ok(None),
            }
        }

        match self.parse_message(src, settings)
            .map_err(DecoderError::Error)?
        {
            Async::Ready((msg, decoder)) => {
                if let Some(decoder) = decoder {
                    self.decoder = Some(decoder);
                    Ok(Some(Message::Message {
                        msg,
                        payload: true,
                    }))
                } else {
                    Ok(Some(Message::Message {
                        msg,
                        payload: false,
                    }))
                }
            }
            Async::NotReady => {
                if src.len() >= MAX_BUFFER_SIZE {
                    error!("MAX_BUFFER_SIZE unprocessed data reached, closing");
                    Err(DecoderError::Error(ParseError::TooLarge))
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn parse_message<H>(
        &self, buf: &mut BytesMut, settings: &WorkerSettings<H>,
    ) -> Poll<(SharedHttpInnerMessage, Option<EncodingDecoder>), ParseError> {
        // Parse http message
        let mut has_upgrade = false;
        let mut chunked = false;
        let mut content_length = None;

        let msg = {
            let bytes_ptr = buf.as_ref().as_ptr() as usize;
            let mut headers: [httparse::Header; MAX_HEADERS] =
                unsafe { mem::uninitialized() };

            let (len, method, path, version, headers_len) = {
                let b = unsafe {
                    let b: &[u8] = buf;
                    &*(b as *const [u8])
                };
                let mut req = httparse::Request::new(&mut headers);
                match req.parse(b)? {
                    httparse::Status::Complete(len) => {
                        let method = Method::from_bytes(req.method.unwrap().as_bytes())
                            .map_err(|_| ParseError::Method)?;
                        let path = Url::new(Uri::try_from(req.path.unwrap())?);
                        let version = if req.version.unwrap() == 1 {
                            Version::HTTP_11
                        } else {
                            Version::HTTP_10
                        };
                        (len, method, path, version, req.headers.len())
                    }
                    httparse::Status::Partial => return Ok(Async::NotReady),
                }
            };

            let slice = buf.split_to(len).freeze();

            // convert headers
            let msg = settings.get_http_message();
            {
                let msg_mut = msg.get_mut();
                msg_mut
                    .flags
                    .set(MessageFlags::KEEPALIVE, version != Version::HTTP_10);

                for header in headers[..headers_len].iter() {
                    if let Ok(name) = HeaderName::from_bytes(header.name.as_bytes()) {
                        has_upgrade = has_upgrade || name == header::UPGRADE;

                        let v_start = header.value.as_ptr() as usize - bytes_ptr;
                        let v_end = v_start + header.value.len();
                        let value = unsafe {
                            HeaderValue::from_shared_unchecked(
                                slice.slice(v_start, v_end),
                            )
                        };
                        match name {
                            header::CONTENT_LENGTH => {
                                if let Ok(s) = value.to_str() {
                                    if let Ok(len) = s.parse::<u64>() {
                                        content_length = Some(len)
                                    } else {
                                        debug!("illegal Content-Length: {:?}", len);
                                        return Err(ParseError::Header);
                                    }
                                } else {
                                    debug!("illegal Content-Length: {:?}", len);
                                    return Err(ParseError::Header);
                                }
                            }
                            // transfer-encoding
                            header::TRANSFER_ENCODING => {
                                if let Ok(s) = value.to_str() {
                                    chunked = s.to_lowercase().contains("chunked");
                                } else {
                                    return Err(ParseError::Header);
                                }
                            }
                            // connection keep-alive state
                            header::CONNECTION => {
                                let ka = if let Ok(conn) = value.to_str() {
                                    if version == Version::HTTP_10
                                        && conn.contains("keep-alive")
                                    {
                                        true
                                    } else {
                                        version == Version::HTTP_11
                                            && !(conn.contains("close")
                                                || conn.contains("upgrade"))
                                    }
                                } else {
                                    false
                                };
                                msg_mut.flags.set(MessageFlags::KEEPALIVE, ka);
                            }
                            _ => (),
                        }

                        msg_mut.headers.append(name, value);
                    } else {
                        return Err(ParseError::Header);
                    }
                }

                msg_mut.url = path;
                msg_mut.method = method;
                msg_mut.version = version;
            }
            msg
        };

        // https://tools.ietf.org/html/rfc7230#section-3.3.3
        let decoder = if chunked {
            // Chunked encoding
            Some(EncodingDecoder::chunked())
        } else if let Some(len) = content_length {
            // Content-Length
            Some(EncodingDecoder::length(len))
        } else if has_upgrade || msg.get_ref().method == Method::CONNECT {
            // upgrade(websocket) or connect
            Some(EncodingDecoder::eof())
        } else {
            None
        };

        Ok(Async::Ready((msg, decoder)))
    }
}

/// Decoders to handle different Transfer-Encodings.
///
/// If a message body does not include a Transfer-Encoding, it *should*
/// include a Content-Length header.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodingDecoder {
    kind: Kind,
}

impl EncodingDecoder {
    pub fn length(x: u64) -> EncodingDecoder {
        EncodingDecoder {
            kind: Kind::Length(x),
        }
    }

    pub fn chunked() -> EncodingDecoder {
        EncodingDecoder {
            kind: Kind::Chunked(ChunkedState::Size, 0),
        }
    }

    pub fn eof() -> EncodingDecoder {
        EncodingDecoder {
            kind: Kind::Eof(false),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// A Reader used when a Content-Length header is passed with a positive
    /// integer.
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

impl EncodingDecoder {
    pub fn decode(&mut self, body: &mut BytesMut) -> Poll<Option<Bytes>, io::Error> {
        match self.kind {
            Kind::Length(ref mut remaining) => {
                if *remaining == 0 {
                    Ok(Async::Ready(None))
                } else {
                    if body.is_empty() {
                        return Ok(Async::NotReady);
                    }
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
            Kind::Chunked(ref mut state, ref mut size) => {
                loop {
                    let mut buf = None;
                    // advances the chunked state
                    *state = try_ready!(state.step(body, size, &mut buf));
                    if *state == ChunkedState::End {
                        trace!("End of chunked stream");
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
            Kind::Eof(ref mut is_eof) => {
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
            let b = $rdr[0];
            $rdr.split_to(1);
            b
        } else {
            return Ok(Async::NotReady)
        }
    })
);

impl ChunkedState {
    fn step(
        &self, body: &mut BytesMut, size: &mut u64, buf: &mut Option<Bytes>,
    ) -> Poll<ChunkedState, io::Error> {
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
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid chunk size line: Invalid Size",
                ));
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
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk size linear white space",
            )),
        }
    }
    fn read_extension(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\r' => Ok(Async::Ready(ChunkedState::SizeLf)),
            _ => Ok(Async::Ready(ChunkedState::Extension)), // no supported extensions
        }
    }
    fn read_size_lf(
        rdr: &mut BytesMut, size: &mut u64,
    ) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\n' if *size > 0 => Ok(Async::Ready(ChunkedState::Body)),
            b'\n' if *size == 0 => Ok(Async::Ready(ChunkedState::EndCr)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk size LF",
            )),
        }
    }

    fn read_body(
        rdr: &mut BytesMut, rem: &mut u64, buf: &mut Option<Bytes>,
    ) -> Poll<ChunkedState, io::Error> {
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
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk body CR",
            )),
        }
    }
    fn read_body_lf(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\n' => Ok(Async::Ready(ChunkedState::Size)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk body LF",
            )),
        }
    }
    fn read_end_cr(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\r' => Ok(Async::Ready(ChunkedState::EndLf)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk end CR",
            )),
        }
    }
    fn read_end_lf(rdr: &mut BytesMut) -> Poll<ChunkedState, io::Error> {
        match byte!(rdr) {
            b'\n' => Ok(Async::Ready(ChunkedState::End)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk end LF",
            )),
        }
    }
}
