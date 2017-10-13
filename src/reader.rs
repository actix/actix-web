use std::{self, io, ptr};

use httparse;
use http::{Method, Version, Uri, HttpTryFrom, HeaderMap};
use http::header::{self, HeaderName, HeaderValue};
use bytes::{BytesMut, BufMut};
use futures::{Async, Poll};
use tokio_io::AsyncRead;

use error::ParseError;
use decode::Decoder;
use httpmessage::HttpRequest;
use payload::{Payload, PayloadError, PayloadSender};

const MAX_HEADERS: usize = 100;
const INIT_BUFFER_SIZE: usize = 8192;
const MAX_BUFFER_SIZE: usize = 131_072;

struct PayloadInfo {
    tx: PayloadSender,
    decoder: Decoder,
}

pub(crate) struct Reader {
    read_buf: BytesMut,
    payload: Option<PayloadInfo>,
}

enum Decoding {
    Paused,
    Ready,
    NotReady,
}

pub(crate) enum ReaderError {
    Payload,
    Error(ParseError),
}

impl Reader {
    pub fn new() -> Reader {
        Reader {
            read_buf: BytesMut::new(),
            payload: None,
        }
    }

    #[allow(dead_code)]
    pub fn consume_leading_lines(&mut self) {
        if !self.read_buf.is_empty() {
            let mut i = 0;
            while i < self.read_buf.len() {
                match self.read_buf[i] {
                    b'\r' | b'\n' => i += 1,
                    _ => break,
                }
            }            self.read_buf.split_to(i);
        }
    }

    fn decode(&mut self) -> std::result::Result<Decoding, ReaderError>
    {
        if let Some(ref mut payload) = self.payload {
            if payload.tx.maybe_paused() {
                return Ok(Decoding::Paused)
            }
            loop {
                match payload.decoder.decode(&mut self.read_buf) {
                    Ok(Async::Ready(Some(bytes))) => {
                        payload.tx.feed_data(bytes)
                    },
                    Ok(Async::Ready(None)) => {
                        payload.tx.feed_eof();
                        return Ok(Decoding::Ready)
                    },
                    Ok(Async::NotReady) => return Ok(Decoding::NotReady),
                    Err(err) => {
                        payload.tx.set_error(err.into());
                        return Err(ReaderError::Payload)
                    }
                }
            }
        } else {
            return Ok(Decoding::Ready)
        }
    }
    
    pub fn parse<T>(&mut self, io: &mut T) -> Poll<(HttpRequest, Payload), ReaderError>
        where T: AsyncRead
    {
        loop {
            match self.decode()? {
                Decoding::Paused => return Ok(Async::NotReady),
                Decoding::Ready => {
                    self.payload = None;
                    break
                },
                Decoding::NotReady => {
                    match self.read_from_io(io) {
                        Ok(Async::Ready(0)) => {
                            if let Some(ref mut payload) = self.payload {
                                payload.tx.set_error(PayloadError::Incomplete);
                            }
                            // http channel should deal with payload errors
                            return Err(ReaderError::Payload)
                        }
                        Ok(Async::Ready(_)) => {
                            continue
                        }
                        Ok(Async::NotReady) => break,
                        Err(err) => {
                            if let Some(ref mut payload) = self.payload {
                                payload.tx.set_error(err.into());
                            }
                            // http channel should deal with payload errors
                            return Err(ReaderError::Payload)
                        }
                    }
                }
            }
        }

        loop {
            match try!(parse(&mut self.read_buf).map_err(ReaderError::Error)) {
                Some((msg, decoder)) => {
                    let payload = if let Some(decoder) = decoder {
                        let (tx, rx) = Payload::new(false);
                        let payload = PayloadInfo {
                            tx: tx,
                            decoder: decoder,
                        };
                        self.payload = Some(payload);

                        loop {
                            match self.decode()? {
                                Decoding::Paused =>
                                    break,
                                Decoding::Ready => {
                                    self.payload = None;
                                    break
                                },
                                Decoding::NotReady => {
                                    match self.read_from_io(io) {
                                        Ok(Async::Ready(0)) => {
                                            trace!("parse eof");
                                            if let Some(ref mut payload) = self.payload {
                                                payload.tx.set_error(PayloadError::Incomplete);
                                            }
                                            // http channel should deal with payload errors
                                            return Err(ReaderError::Payload)
                                        }
                                        Ok(Async::Ready(_)) => {
                                            continue
                                        }
                                        Ok(Async::NotReady) => break,
                                        Err(err) => {
                                            if let Some(ref mut payload) = self.payload {
                                                payload.tx.set_error(err.into());
                                            }
                                            // http channel should deal with payload errors
                                            return Err(ReaderError::Payload)
                                        }
                                    }
                                }
                            }
                        }
                        rx
                    } else {
                        let (_, rx) = Payload::new(true);
                        rx
                    };
                    return Ok(Async::Ready((msg, payload)));
                },
                None => {
                    if self.read_buf.capacity() >= MAX_BUFFER_SIZE {
                        debug!("MAX_BUFFER_SIZE reached, closing");
                        return Err(ReaderError::Error(ParseError::TooLarge));
                    }
                },
            }
            match self.read_from_io(io) {
                Ok(Async::Ready(0)) => {
                    trace!("parse eof");
                    return Err(ReaderError::Error(ParseError::Incomplete));
                },
                Ok(Async::Ready(_)) => (),
                Ok(Async::NotReady) =>
                    return Ok(Async::NotReady),
                Err(err) =>
                    return Err(ReaderError::Error(err.into()))
            }
        }
    }

    fn read_from_io<T: AsyncRead>(&mut self, io: &mut T) -> Poll<usize, io::Error> {
        if self.read_buf.remaining_mut() < INIT_BUFFER_SIZE {
            self.read_buf.reserve(INIT_BUFFER_SIZE);
            unsafe { // Zero out unused memory
                let buf = self.read_buf.bytes_mut();
                let len = buf.len();
                ptr::write_bytes(buf.as_mut_ptr(), 0, len);
            }
        }
        unsafe {
            let n = match io.read(self.read_buf.bytes_mut()) {
                Ok(n) => n,
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        return Ok(Async::NotReady);
                    }
                    return Err(e)
                }
            };
            self.read_buf.advance_mut(n);
            Ok(Async::Ready(n))
        }
    }
}


pub fn parse(buf: &mut BytesMut) -> Result<Option<(HttpRequest, Option<Decoder>)>, ParseError>
{
    if buf.is_empty() {
        return Ok(None);
    }

    // Parse http message
    let mut headers_indices = [HeaderIndices {
        name: (0, 0),
        value: (0, 0)
    }; MAX_HEADERS];

    let (len, method, path, version, headers_len) = {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        trace!("Request.parse([Header; {}], [u8; {}])", headers.len(), buf.len());
        let mut req = httparse::Request::new(&mut headers);
        match try!(req.parse(buf)) {
            httparse::Status::Complete(len) => {
                trace!("Request.parse Complete({})", len);
                let method = Method::try_from(req.method.unwrap())
                    .map_err(|_| ParseError::Method)?;
                let path = req.path.unwrap();
                let bytes_ptr = buf.as_ref().as_ptr() as usize;
                let path_start = path.as_ptr() as usize - bytes_ptr;
                let path_end = path_start + path.len();
                let path = (path_start, path_end);

                let version = if req.version.unwrap() == 1 {
                    Version::HTTP_11
                } else {
                    Version::HTTP_10
                };

                record_header_indices(buf.as_ref(), req.headers, &mut headers_indices);
                let headers_len = req.headers.len();
                (len, method, path, version, headers_len)
            }
            httparse::Status::Partial => return Ok(None),
        }
    };

    let slice = buf.split_to(len).freeze();
    let path = slice.slice(path.0, path.1);
    // path was found to be utf8 by httparse
    let uri = Uri::from_shared(path).map_err(|_| ParseError::Uri)?;

    // convert headers
    let mut headers = HeaderMap::with_capacity(headers_len);
    for header in headers_indices[..headers_len].iter() {
        if let Ok(name) = HeaderName::try_from(slice.slice(header.name.0, header.name.1)) {
            if let Ok(value) = HeaderValue::try_from(
                slice.slice(header.value.0, header.value.1))
            {
                headers.insert(name, value);
            } else {
                return Err(ParseError::Header)
            }
        } else {
            return Err(ParseError::Header)
        }
    }

    let msg = HttpRequest::new(method, uri, version, headers);
    let upgrade = msg.is_upgrade() || *msg.method() == Method::CONNECT;
    let chunked = msg.is_chunked()?;

    let decoder = if upgrade {
        Some(Decoder::eof())
    }
    // Content-Length
    else if let Some(len) = msg.headers().get(header::CONTENT_LENGTH) {
        if chunked {
            return Err(ParseError::Header)
        }
        if let Ok(s) = len.to_str() {
            if let Ok(len) = s.parse::<u64>() {
                Some(Decoder::length(len))
            } else {
                debug!("illegal Content-Length: {:?}", len);
                return Err(ParseError::Header)
            }
        } else {
            debug!("illegal Content-Length: {:?}", len);
            return Err(ParseError::Header)
        }
    } else if chunked {
        Some(Decoder::chunked())
    } else {
        None
    };
    Ok(Some((msg, decoder)))
}

#[derive(Clone, Copy)]
struct HeaderIndices {
    name: (usize, usize),
    value: (usize, usize),
}

fn record_header_indices(bytes: &[u8],
                         headers: &[httparse::Header],
                         indices: &mut [HeaderIndices])
{
    let bytes_ptr = bytes.as_ptr() as usize;
    for (header, indices) in headers.iter().zip(indices.iter_mut()) {
        let name_start = header.name.as_ptr() as usize - bytes_ptr;
        let name_end = name_start + header.name.len();
        indices.name = (name_start, name_end);
        let value_start = header.value.as_ptr() as usize - bytes_ptr;
        let value_end = value_start + header.value.len();
        indices.value = (value_start, value_end);
    }
}
