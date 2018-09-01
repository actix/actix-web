use std::mem;

use bytes::{Bytes, BytesMut};
use futures::{Async, Poll};
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, StatusCode, Version};
use httparse;

use error::{ParseError, PayloadError};

use server::h1decoder::{EncodingDecoder, HeaderIndex};
use server::IoStream;

use super::response::ClientMessage;
use super::ClientResponse;

const MAX_BUFFER_SIZE: usize = 131_072;
const MAX_HEADERS: usize = 96;

#[derive(Default)]
pub struct HttpResponseParser {
    decoder: Option<EncodingDecoder>,
}

#[derive(Debug, Fail)]
pub enum HttpResponseParserError {
    /// Server disconnected
    #[fail(display = "Server disconnected")]
    Disconnect,
    #[fail(display = "{}", _0)]
    Error(#[cause] ParseError),
}

impl HttpResponseParser {
    pub fn parse<T>(
        &mut self, io: &mut T, buf: &mut BytesMut,
    ) -> Poll<ClientResponse, HttpResponseParserError>
    where
        T: IoStream,
    {
        loop {
            // Don't call parser until we have data to parse.
            if !buf.is_empty() {
                match HttpResponseParser::parse_message(buf)
                    .map_err(HttpResponseParserError::Error)?
                {
                    Async::Ready((msg, decoder)) => {
                        self.decoder = decoder;
                        return Ok(Async::Ready(msg));
                    }
                    Async::NotReady => {
                        if buf.capacity() >= MAX_BUFFER_SIZE {
                            return Err(HttpResponseParserError::Error(ParseError::TooLarge));
                        }
                        // Parser needs more data.
                    }
                }
            }
            // Read some more data into the buffer for the parser.
            match io.read_available(buf) {
                Ok(Async::Ready((false, true))) => {
                    return Err(HttpResponseParserError::Disconnect)
                }
                Ok(Async::Ready(_)) => (),
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(err) => {
                    return Err(HttpResponseParserError::Error(err.into()))
                }
            }
        }
    }

    pub fn parse_payload<T>(
        &mut self, io: &mut T, buf: &mut BytesMut,
    ) -> Poll<Option<Bytes>, PayloadError>
    where
        T: IoStream,
    {
        if self.decoder.is_some() {
            loop {
                // read payload
                let (not_ready, stream_finished) = match io.read_available(buf) {
                    Ok(Async::Ready((_, true))) => (false, true),
                    Ok(Async::Ready((_, false))) => (false, false),
                    Ok(Async::NotReady) => (true, false),
                    Err(err) => return Err(err.into()),
                };

                match self.decoder.as_mut().unwrap().decode(buf) {
                    Ok(Async::Ready(Some(b))) => return Ok(Async::Ready(Some(b))),
                    Ok(Async::Ready(None)) => {
                        self.decoder.take();
                        return Ok(Async::Ready(None));
                    }
                    Ok(Async::NotReady) => {
                        if not_ready {
                            return Ok(Async::NotReady);
                        }
                        if stream_finished {
                            return Err(PayloadError::Incomplete);
                        }
                    }
                    Err(err) => return Err(err.into()),
                }
            }
        } else {
            Ok(Async::Ready(None))
        }
    }

    fn parse_message(
        buf: &mut BytesMut,
    ) -> Poll<(ClientResponse, Option<EncodingDecoder>), ParseError> {
        // Unsafe: we read only this data only after httparse parses headers into.
        // performance bump for pipeline benchmarks.
        let mut headers: [HeaderIndex; MAX_HEADERS] = unsafe { mem::uninitialized() };

        let (len, version, status, headers_len) = {
            let mut parsed: [httparse::Header; MAX_HEADERS] =
                unsafe { mem::uninitialized() };

            let mut resp = httparse::Response::new(&mut parsed);
            match resp.parse(buf)? {
                httparse::Status::Complete(len) => {
                    let version = if resp.version.unwrap_or(1) == 1 {
                        Version::HTTP_11
                    } else {
                        Version::HTTP_10
                    };
                    HeaderIndex::record(buf, resp.headers, &mut headers);
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
        for idx in headers[..headers_len].iter() {
            if let Ok(name) = HeaderName::from_bytes(&slice[idx.name.0..idx.name.1]) {
                // Unsafe: httparse check header value for valid utf-8
                let value = unsafe {
                    HeaderValue::from_shared_unchecked(
                        slice.slice(idx.value.0, idx.value.1),
                    )
                };
                hdrs.append(name, value);
            } else {
                return Err(ParseError::Header);
            }
        }

        let decoder = if status == StatusCode::SWITCHING_PROTOCOLS {
            Some(EncodingDecoder::eof())
        } else if let Some(len) = hdrs.get(header::CONTENT_LENGTH) {
            // Content-Length
            if let Ok(s) = len.to_str() {
                if let Ok(len) = s.parse::<u64>() {
                    Some(EncodingDecoder::length(len))
                } else {
                    debug!("illegal Content-Length: {:?}", len);
                    return Err(ParseError::Header);
                }
            } else {
                debug!("illegal Content-Length: {:?}", len);
                return Err(ParseError::Header);
            }
        } else if chunked(&hdrs)? {
            // Chunked encoding
            Some(EncodingDecoder::chunked())
        } else {
            None
        };

        if let Some(decoder) = decoder {
            Ok(Async::Ready((
                ClientResponse::new(ClientMessage {
                    status,
                    version,
                    headers: hdrs,
                    cookies: None,
                }),
                Some(decoder),
            )))
        } else {
            Ok(Async::Ready((
                ClientResponse::new(ClientMessage {
                    status,
                    version,
                    headers: hdrs,
                    cookies: None,
                }),
                None,
            )))
        }
    }
}

/// Check if request has chunked transfer encoding
pub fn chunked(headers: &HeaderMap) -> Result<bool, ParseError> {
    if let Some(encodings) = headers.get(header::TRANSFER_ENCODING) {
        if let Ok(s) = encodings.to_str() {
            Ok(s.to_lowercase().contains("chunked"))
        } else {
            Err(ParseError::Header)
        }
    } else {
        Ok(false)
    }
}
