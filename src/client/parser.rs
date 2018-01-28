use std::mem;
use httparse;
use http::{Version, HttpTryFrom, HeaderMap, StatusCode};
use http::header::{self, HeaderName, HeaderValue};
use bytes::BytesMut;
use futures::{Poll, Async};

use error::{ParseError, PayloadError};
use payload::{Payload, PayloadWriter, DEFAULT_BUFFER_SIZE};

use server::{utils, IoStream};
use server::h1::{Decoder, chunked};
use server::encoding::PayloadType;

use super::ClientResponse;

const MAX_BUFFER_SIZE: usize = 131_072;
const MAX_HEADERS: usize = 96;


pub struct HttpResponseParser {
    payload: Option<PayloadInfo>,
}

enum Decoding {
    Paused,
    Ready,
    NotReady,
}

struct PayloadInfo {
    tx: PayloadType,
    decoder: Decoder,
}

#[derive(Debug)]
pub enum HttpResponseParserError {
    Disconnect,
    Payload,
    Error(ParseError),
}

impl HttpResponseParser {
    pub fn new() -> HttpResponseParser {
        HttpResponseParser {
            payload: None,
        }
    }

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Decoding, HttpResponseParserError> {
        if let Some(ref mut payload) = self.payload {
            if payload.tx.capacity() > DEFAULT_BUFFER_SIZE {
                return Ok(Decoding::Paused)
            }
            loop {
                match payload.decoder.decode(buf) {
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
                        return Err(HttpResponseParserError::Payload)
                    }
                }
            }
        } else {
            return Ok(Decoding::Ready)
        }
    }

    pub fn parse<T>(&mut self, io: &mut T, buf: &mut BytesMut)
                    -> Poll<ClientResponse, HttpResponseParserError>
        where T: IoStream
    {
        // read payload
        if self.payload.is_some() {
            match utils::read_from_io(io, buf) {
                Ok(Async::Ready(0)) => {
                    if let Some(ref mut payload) = self.payload {
                        payload.tx.set_error(PayloadError::Incomplete);
                    }
                    // http channel should not deal with payload errors
                    return Err(HttpResponseParserError::Payload)
                },
                Err(err) => {
                    if let Some(ref mut payload) = self.payload {
                        payload.tx.set_error(err.into());
                    }
                    // http channel should not deal with payload errors
                    return Err(HttpResponseParserError::Payload)
                }
                _ => (),
            }
            match self.decode(buf)? {
                Decoding::Ready => self.payload = None,
                Decoding::Paused | Decoding::NotReady => return Ok(Async::NotReady),
            }
        }

        // if buf is empty parse_message will always return NotReady, let's avoid that
        let read = if buf.is_empty() {
            match utils::read_from_io(io, buf) {
                Ok(Async::Ready(0)) => {
                    // debug!("Ignored premature client disconnection");
                    return Err(HttpResponseParserError::Disconnect);
                },
                Ok(Async::Ready(_)) => (),
                Ok(Async::NotReady) =>
                    return Ok(Async::NotReady),
                Err(err) =>
                    return Err(HttpResponseParserError::Error(err.into()))
            }
            false
        } else {
            true
        };

        loop {
            match HttpResponseParser::parse_message(buf).map_err(HttpResponseParserError::Error)? {
                Async::Ready((msg, decoder)) => {
                    // process payload
                    if let Some(payload) = decoder {
                        self.payload = Some(payload);
                        match self.decode(buf)? {
                            Decoding::Paused | Decoding::NotReady => (),
                            Decoding::Ready => self.payload = None,
                        }
                    }
                    return Ok(Async::Ready(msg));
                },
                Async::NotReady => {
                    if buf.capacity() >= MAX_BUFFER_SIZE {
                        error!("MAX_BUFFER_SIZE unprocessed data reached, closing");
                        return Err(HttpResponseParserError::Error(ParseError::TooLarge));
                    }
                    if read {
                        match utils::read_from_io(io, buf) {
                            Ok(Async::Ready(0)) => {
                                debug!("Ignored premature client disconnection");
                                return Err(HttpResponseParserError::Disconnect);
                            },
                            Ok(Async::Ready(_)) => (),
                            Ok(Async::NotReady) =>
                                return Ok(Async::NotReady),
                            Err(err) =>
                                return Err(HttpResponseParserError::Error(err.into()))
                        }
                    } else {
                        return Ok(Async::NotReady)
                    }
                },
            }
        }
    }

    fn parse_message(buf: &mut BytesMut) -> Poll<(ClientResponse, Option<PayloadInfo>), ParseError>
    {
        // Parse http message
        let bytes_ptr = buf.as_ref().as_ptr() as usize;
        let mut headers: [httparse::Header; MAX_HEADERS] =
            unsafe{mem::uninitialized()};

        let (len, version, status, headers_len) = {
            let b = unsafe{ let b: &[u8] = buf; mem::transmute(b) };
            let mut resp = httparse::Response::new(&mut headers);
            match resp.parse(b)? {
                httparse::Status::Complete(len) => {
                    let version = if resp.version.unwrap() == 1 {
                        Version::HTTP_11
                    } else {
                        Version::HTTP_10
                    };
                    let status = StatusCode::from_u16(resp.code.unwrap())
                        .map_err(|_| ParseError::Status)?;

                    (len, version, status, resp.headers.len())
                }
                httparse::Status::Partial => return Ok(Async::NotReady),
            }
        };


        let slice = buf.split_to(len).freeze();

        // convert headers
        let mut hdrs = HeaderMap::new();
        for header in headers[..headers_len].iter() {
            if let Ok(name) = HeaderName::try_from(header.name) {
                let v_start = header.value.as_ptr() as usize - bytes_ptr;
                let v_end = v_start + header.value.len();
                let value = unsafe {
                    HeaderValue::from_shared_unchecked(slice.slice(v_start, v_end)) };
                hdrs.append(name, value);
            } else {
                return Err(ParseError::Header)
            }
        }

        let decoder = if let Some(len) = hdrs.get(header::CONTENT_LENGTH) {
            // Content-Length
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
        } else if chunked(&hdrs)? {
            // Chunked encoding
            Some(Decoder::chunked())
        } else if hdrs.contains_key(header::UPGRADE) {
            Some(Decoder::eof())
        } else {
            None
        };

        if let Some(decoder) = decoder {
            let (psender, payload) = Payload::new(false);
            let info = PayloadInfo {
                tx: PayloadType::new(&hdrs, psender),
                decoder: decoder,
            };
            Ok(Async::Ready(
                (ClientResponse::new(status, version, hdrs, Some(payload)), Some(info))))
        } else {
            Ok(Async::Ready(
                (ClientResponse::new(status, version, hdrs, None), None)))
        }
    }
}
