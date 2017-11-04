use std::{io, cmp};
use bytes::{Bytes, BytesMut};
use futures::{Async, Poll};
use http2::{Reason, SendStream};
use http2::server::Respond;
use http::{Version, HttpTryFrom, Response};
use http::header::{HeaderValue, CONNECTION, CONTENT_TYPE,
                   CONTENT_LENGTH, TRANSFER_ENCODING, DATE};

use date;
use body::Body;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use h1writer::{Writer, WriterState};

const CHUNK_SIZE: usize = 16_384;
const MAX_WRITE_BUFFER_SIZE: usize = 65_536; // max buffer size 64k


pub(crate) struct H2Writer {
    respond: Respond<Bytes>,
    stream: Option<SendStream<Bytes>>,
    buffer: BytesMut,
    started: bool,
    encoder: Encoder,
    disconnected: bool,
    eof: bool,
}

impl H2Writer {

    pub fn new(respond: Respond<Bytes>) -> H2Writer {
        H2Writer {
            respond: respond,
            stream: None,
            buffer: BytesMut::new(),
            started: false,
            encoder: Encoder::length(0),
            disconnected: false,
            eof: true,
        }
    }

    pub fn reset(&mut self, reason: Reason) {
        if let Some(mut stream) = self.stream.take() {
            stream.send_reset(reason)
        }
    }

    fn write_to_stream(&mut self) -> Result<WriterState, io::Error> {
        if !self.started {
            return Ok(WriterState::Done)
        }

        if let Some(ref mut stream) = self.stream {
            if self.buffer.is_empty() {
                if self.eof {
                    let _ = stream.send_data(Bytes::new(), true);
                }
                return Ok(WriterState::Done)
            }

            loop {
                match stream.poll_capacity() {
                    Ok(Async::NotReady) => {
                        if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
                            return Ok(WriterState::Pause)
                        } else {
                            return Ok(WriterState::Done)
                        }
                    }
                    Ok(Async::Ready(None)) => {
                        return Ok(WriterState::Done)
                    }
                    Ok(Async::Ready(Some(cap))) => {
                        let len = self.buffer.len();
                        let bytes = self.buffer.split_to(cmp::min(cap, len));
                        let eof = self.buffer.is_empty() && self.eof;

                        if let Err(_) = stream.send_data(bytes.freeze(), eof) {
                            return Err(io::Error::new(io::ErrorKind::Other, ""))
                        } else {
                            if !self.buffer.is_empty() {
                                let cap = cmp::min(self.buffer.len(), CHUNK_SIZE);
                                stream.reserve_capacity(cap);
                            } else {
                                return Ok(WriterState::Done)
                            }
                        }
                    }
                    Err(_) => {
                        return Err(io::Error::new(io::ErrorKind::Other, ""))
                    }
                }
            }
        }
        return Ok(WriterState::Done)
    }
}

impl Writer for H2Writer {

    fn start(&mut self, _: &mut HttpRequest, msg: &mut HttpResponse)
             -> Result<WriterState, io::Error>
    {
        trace!("Prepare message status={:?}", msg);

        // prepare response
        self.started = true;
        let body = msg.replace_body(Body::Empty);

        // http2 specific
        msg.headers.remove(CONNECTION);
        msg.headers.remove(TRANSFER_ENCODING);

        match body {
            Body::Empty => {
                if msg.chunked() {
                    error!("Chunked transfer is enabled but body is set to Empty");
                }
                msg.headers.insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
                self.encoder = Encoder::length(0);
            },
            Body::Length(n) => {
                if msg.chunked() {
                    error!("Chunked transfer is enabled but body with specific length is specified");
                }
                self.eof = false;
                msg.headers.insert(
                    CONTENT_LENGTH,
                    HeaderValue::from_str(format!("{}", n).as_str()).unwrap());
                self.encoder = Encoder::length(n);
            },
            Body::Binary(ref bytes) => {
                self.eof = false;
                msg.headers.insert(
                    CONTENT_LENGTH,
                    HeaderValue::from_str(format!("{}", bytes.len()).as_str()).unwrap());
                self.encoder = Encoder::length(0);
            }
            _ => {
                msg.headers.remove(CONTENT_LENGTH);
                self.eof = false;
                self.encoder = Encoder::eof();
            }
        }

        // using http::h1::date is quite a lot faster than generating
        // a unique Date header each time like req/s goes up about 10%
        if !msg.headers.contains_key(DATE) {
            let mut bytes = BytesMut::with_capacity(29);
            date::extend(&mut bytes);
            msg.headers.insert(DATE, HeaderValue::try_from(bytes.freeze()).unwrap());
        }

        // default content-type
        if !msg.headers.contains_key(CONTENT_TYPE) {
            msg.headers.insert(
                CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
        }

        let mut resp = Response::new(());
        *resp.status_mut() = msg.status;
        *resp.version_mut() = Version::HTTP_2;
        for (key, value) in msg.headers().iter() {
            resp.headers_mut().insert(key, value.clone());
        }

        match self.respond.send_response(resp, self.eof) {
            Ok(stream) => {
                self.stream = Some(stream);
            }
            Err(_) => {
                return Err(io::Error::new(io::ErrorKind::Other, "err"))
            }
        }

        if let Body::Binary(ref bytes) = body {
            self.eof = true;
            self.buffer.extend_from_slice(bytes.as_ref());
            if let Some(ref mut stream) = self.stream {
                stream.reserve_capacity(cmp::min(self.buffer.len(), CHUNK_SIZE));
            }
            return Ok(WriterState::Done)
        }
        msg.replace_body(body);

        Ok(WriterState::Done)
    }

    fn write(&mut self, payload: &[u8]) -> Result<WriterState, io::Error> {
        if !self.disconnected {
            if self.started {
                // TODO: add warning, write after EOF
                self.encoder.encode(&mut self.buffer, payload);
            } else {
                // might be response for EXCEPT
                self.buffer.extend_from_slice(payload)
            }
        }

        if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
            return Ok(WriterState::Pause)
        } else {
            return Ok(WriterState::Done)
        }
    }

    fn write_eof(&mut self) -> Result<WriterState, io::Error> {
        self.eof = true;
        if !self.encoder.encode_eof(&mut self.buffer) {
            Err(io::Error::new(io::ErrorKind::Other,
                               "Last payload item, but eof is not reached"))
        } else {
            if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
                return Ok(WriterState::Pause)
            } else {
                return Ok(WriterState::Done)
            }
        }
    }

    fn poll_complete(&mut self) -> Poll<(), io::Error> {
        match self.write_to_stream() {
            Ok(WriterState::Done) => Ok(Async::Ready(())),
            Ok(WriterState::Pause) => Ok(Async::NotReady),
            Err(err) => Err(err)
        }
    }
}


/// Encoders to handle different Transfer-Encodings.
#[derive(Debug, Clone)]
pub(crate) struct Encoder {
    kind: Kind,
}

#[derive(Debug, PartialEq, Clone)]
enum Kind {
    /// An Encoder for when Content-Length is set.
    ///
    /// Enforces that the body is not longer than the Content-Length header.
    Length(u64),
    /// An Encoder for when Content-Length is not known.
    ///
    /// Appliction decides when to stop writing.
    Eof,
}

impl Encoder {

    pub fn eof() -> Encoder {
        Encoder {
            kind: Kind::Eof,
        }
    }

    pub fn length(len: u64) -> Encoder {
        Encoder {
            kind: Kind::Length(len),
        }
    }

    /// Encode message. Return `EOF` state of encoder
    pub fn encode(&mut self, dst: &mut BytesMut, msg: &[u8]) -> bool {
        match self.kind {
            Kind::Eof => {
                dst.extend(msg);
                msg.is_empty()
            },
            Kind::Length(ref mut remaining) => {
                if msg.is_empty() {
                    return *remaining == 0
                }
                let max = cmp::min(*remaining, msg.len() as u64);
                trace!("sized write = {}", max);
                dst.extend(msg[..max as usize].as_ref());

                *remaining -= max as u64;
                trace!("encoded {} bytes, remaining = {}", max, remaining);
                *remaining == 0
            },
        }
    }

    /// Encode eof. Return `EOF` state of encoder
    pub fn encode_eof(&mut self, _dst: &mut BytesMut) -> bool {
        match self.kind {
            Kind::Eof => true,
            Kind::Length(ref mut remaining) => {
                return *remaining == 0
            },
        }
    }
}
