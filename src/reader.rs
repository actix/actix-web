use std::{self, fmt, io, ptr};

use httparse;
use http::{Method, Version, Uri, HttpTryFrom};
use bytes::{Bytes, BytesMut, BufMut};
use futures::{Async, Poll};
use tokio_io::AsyncRead;

use hyper::header::{Headers, ContentLength};

use error::{Error, Result};
use decode::Decoder;
use payload::{Payload, PayloadSender};
use httpmessage::{Message, HttpRequest};

const MAX_HEADERS: usize = 100;
const INIT_BUFFER_SIZE: usize = 8192;
const MAX_BUFFER_SIZE: usize = 131_072;

struct PayloadInfo {
    tx: PayloadSender,
    decoder: Decoder,
}

pub struct Reader {
    read_buf: BytesMut,
    payload: Option<PayloadInfo>,
}

enum Decoding {
    Paused,
    Ready,
    NotReady,
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
            }
            self.read_buf.split_to(i);
        }
    }

    fn decode(&mut self) -> std::result::Result<Decoding, Error>
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
                    Err(_) => return Err(Error::Incomplete),
                }
            }
        } else {
            return Ok(Decoding::Ready)
        }
    }
    
    pub fn parse<T>(&mut self, io: &mut T) -> Poll<(HttpRequest, Payload), Error>
        where T: AsyncRead
    {


        loop {
            match self.decode()? {
                Decoding::Paused => return Ok(Async::NotReady),
                Decoding::Ready => {
                    println!("decode ready");
                    self.payload = None;
                    break
                },
                Decoding::NotReady => {
                    if 0 == try_ready!(self.read_from_io(io)) {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof, ParseEof).into());
                    }
                }
            }
        }

        loop {
            match try!(parse(&mut self.read_buf)) {
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
                                    println!("decoded 3");
                                    self.payload = None;
                                    break
                                },
                                Decoding::NotReady => {
                                    match self.read_from_io(io) {
                                        Ok(Async::Ready(0)) => {
                                            trace!("parse eof");
                                            return Err(io::Error::new(
                                                io::ErrorKind::UnexpectedEof, ParseEof).into());
                                        }
                                        Ok(Async::Ready(_)) => {
                                            continue
                                        }
                                        Ok(Async::NotReady) => break,
                                        Err(err) => return Err(err.into()),
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
                        return Err(Error::TooLarge);
                    }
                },
            }
            if 0 == try_ready!(self.read_from_io(io)) {
                trace!("parse eof");
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, ParseEof).into());
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

#[derive(Debug)]
struct ParseEof;

impl fmt::Display for ParseEof {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("parse eof")
    }
}

impl ::std::error::Error for ParseEof {
    fn description(&self) -> &str {
        "parse eof"
    }
}


pub fn parse(buf: &mut BytesMut) -> Result<Option<(HttpRequest, Option<Decoder>)>> {
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
                let method = Method::try_from(req.method.unwrap()).map_err(|_| Error::Method)?;
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

    let mut headers = Headers::with_capacity(headers_len);
    let slice = buf.split_to(len).freeze();
    let path = slice.slice(path.0, path.1);
    // path was found to be utf8 by httparse
    let uri = Uri::from_shared(path).map_err(|_| Error::Uri)?;

    headers.extend(HeadersAsBytesIter {
        headers: headers_indices[..headers_len].iter(),
        slice: slice,
    });

    let msg = HttpRequest::new(method, uri, version, headers);
    let upgrade = msg.is_upgrade() || *msg.method() == Method::CONNECT;
    let chunked = msg.is_chunked()?;

    if upgrade {
        Ok(Some((msg, Some(Decoder::eof()))))
    }
    // Content-Length
    else if let Some(&ContentLength(len)) = msg.headers().get() {
        if chunked {
            return Err(Error::Header)
        }
        Ok(Some((msg, Some(Decoder::length(len)))))
    } else if msg.headers().has::<ContentLength>() {
        debug!("illegal Content-Length: {:?}", msg.headers().get_raw("Content-Length"));
        Err(Error::Header)
    } else if chunked {
        Ok(Some((msg, Some(Decoder::chunked()))))
    } else {
        Ok(Some((msg, None)))
    }
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

struct HeadersAsBytesIter<'a> {
    headers: ::std::slice::Iter<'a, HeaderIndices>,
    slice: Bytes,
}

impl<'a> Iterator for HeadersAsBytesIter<'a> {
    type Item = (&'a str, Bytes);
    fn next(&mut self) -> Option<Self::Item> {
        self.headers.next().map(|header| {
            let name = unsafe {
                let bytes = ::std::slice::from_raw_parts(
                    self.slice.as_ref().as_ptr().offset(header.name.0 as isize),
                    header.name.1 - header.name.0
                );
                ::std::str::from_utf8_unchecked(bytes)
            };
            (name, self.slice.slice(header.value.0, header.value.1))
        })
    }
}
