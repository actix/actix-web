use std::{io, cmp};
use bytes::{Bytes, BytesMut};
use futures::{Async, Poll};
use http2::{Reason, SendStream};
use http2::server::SendResponse;
use http::{Version, HttpTryFrom, Response};
use http::header::{HeaderValue, CONNECTION, TRANSFER_ENCODING, DATE, CONTENT_LENGTH};

use helpers;
use body::{Body, Binary};
use helpers::SharedBytes;
use httprequest::HttpMessage;
use httpresponse::HttpResponse;
use super::encoding::PayloadEncoder;
use super::{Writer, WriterState, MAX_WRITE_BUFFER_SIZE};

const CHUNK_SIZE: usize = 16_384;

bitflags! {
    struct Flags: u8 {
        const STARTED = 0b0000_0001;
        const DISCONNECTED = 0b0000_0010;
        const EOF = 0b0000_0100;
    }
}

pub(crate) struct H2Writer {
    respond: SendResponse<Bytes>,
    stream: Option<SendStream<Bytes>>,
    encoder: PayloadEncoder,
    flags: Flags,
    written: u64,
    buffer: SharedBytes,
}

impl H2Writer {

    pub fn new(respond: SendResponse<Bytes>, buf: SharedBytes) -> H2Writer {
        H2Writer {
            respond: respond,
            stream: None,
            encoder: PayloadEncoder::empty(buf.clone()),
            flags: Flags::empty(),
            written: 0,
            buffer: buf,
        }
    }

    pub fn reset(&mut self, reason: Reason) {
        if let Some(mut stream) = self.stream.take() {
            stream.send_reset(reason)
        }
    }

    fn write_to_stream(&mut self) -> io::Result<WriterState> {
        if !self.flags.contains(Flags::STARTED) {
            return Ok(WriterState::Done)
        }

        if let Some(ref mut stream) = self.stream {
            let buffer = self.encoder.get_mut();

            if buffer.is_empty() {
                if self.flags.contains(Flags::EOF) {
                    let _ = stream.send_data(Bytes::new(), true);
                }
                return Ok(WriterState::Done)
            }

            loop {
                match stream.poll_capacity() {
                    Ok(Async::NotReady) => {
                        if buffer.len() > MAX_WRITE_BUFFER_SIZE {
                            return Ok(WriterState::Pause)
                        } else {
                            return Ok(WriterState::Done)
                        }
                    }
                    Ok(Async::Ready(None)) => {
                        return Ok(WriterState::Done)
                    }
                    Ok(Async::Ready(Some(cap))) => {
                        let len = buffer.len();
                        let bytes = buffer.split_to(cmp::min(cap, len));
                        let eof = buffer.is_empty() && self.flags.contains(Flags::EOF);
                        self.written += bytes.len() as u64;

                        if let Err(err) = stream.send_data(bytes.freeze(), eof) {
                            return Err(io::Error::new(io::ErrorKind::Other, err))
                        } else if !buffer.is_empty() {
                            let cap = cmp::min(buffer.len(), CHUNK_SIZE);
                            stream.reserve_capacity(cap);
                        } else {
                            return Ok(WriterState::Pause)
                        }
                    }
                    Err(_) => {
                        return Err(io::Error::new(io::ErrorKind::Other, ""))
                    }
                }
            }
        }
        Ok(WriterState::Done)
    }
}

impl Writer for H2Writer {

    fn written(&self) -> u64 {
        self.written
    }

    #[inline]
    fn flush(&mut self) -> Poll<(), io::Error> {
        Ok(Async::Ready(()))
    }

    fn start(&mut self, req: &mut HttpMessage, msg: &mut HttpResponse) -> io::Result<WriterState> {
        // prepare response
        self.flags.insert(Flags::STARTED);
        self.encoder = PayloadEncoder::new(self.buffer.clone(), req, msg);
        if let Body::Empty = *msg.body() {
            self.flags.insert(Flags::EOF);
        }

        // http2 specific
        msg.headers_mut().remove(CONNECTION);
        msg.headers_mut().remove(TRANSFER_ENCODING);

        // using helpers::date is quite a lot faster
        if !msg.headers().contains_key(DATE) {
            let mut bytes = BytesMut::with_capacity(29);
            helpers::date_value(&mut bytes);
            msg.headers_mut().insert(DATE, HeaderValue::try_from(bytes.freeze()).unwrap());
        }

        let body = msg.replace_body(Body::Empty);
        match body {
            Body::Binary(ref bytes) => {
                let mut val = BytesMut::new();
                helpers::convert_usize(bytes.len(), &mut val);
                let l = val.len();
                msg.headers_mut().insert(
                    CONTENT_LENGTH, HeaderValue::try_from(val.split_to(l-2).freeze()).unwrap());
            }
            Body::Empty => {
                msg.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
            },
            _ => (),
        }

        let mut resp = Response::new(());
        *resp.status_mut() = msg.status();
        *resp.version_mut() = Version::HTTP_2;
        for (key, value) in msg.headers().iter() {
            resp.headers_mut().insert(key, value.clone());
        }

        match self.respond.send_response(resp, self.flags.contains(Flags::EOF)) {
            Ok(stream) =>
                self.stream = Some(stream),
            Err(_) =>
                return Err(io::Error::new(io::ErrorKind::Other, "err")),
        }

        trace!("Response: {:?}", msg);

        if let Body::Binary(bytes) = body {
            self.flags.insert(Flags::EOF);
            self.written = bytes.len() as u64;
            self.encoder.write(bytes.as_ref())?;
            if let Some(ref mut stream) = self.stream {
                stream.reserve_capacity(cmp::min(self.encoder.len(), CHUNK_SIZE));
            }
            Ok(WriterState::Pause)
        } else {
            msg.replace_body(body);
            Ok(WriterState::Done)
        }
    }

    fn write(&mut self, payload: Binary) -> io::Result<WriterState> {
        self.written = payload.len() as u64;

        if !self.flags.contains(Flags::DISCONNECTED) {
            if self.flags.contains(Flags::STARTED) {
                // TODO: add warning, write after EOF
                self.encoder.write(payload.as_ref())?;
            } else {
                // might be response for EXCEPT
                self.encoder.get_mut().extend_from_slice(payload.as_ref())
            }
        }

        if self.encoder.len() > MAX_WRITE_BUFFER_SIZE {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    fn write_eof(&mut self) -> io::Result<WriterState> {
        self.encoder.write_eof()?;

        self.flags.insert(Flags::EOF);
        if !self.encoder.is_eof() {
            Err(io::Error::new(io::ErrorKind::Other,
                               "Last payload item, but eof is not reached"))
        } else if self.encoder.len() > MAX_WRITE_BUFFER_SIZE {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    fn poll_completed(&mut self, _shutdown: bool) -> Poll<(), io::Error> {
        match self.write_to_stream() {
            Ok(WriterState::Done) => Ok(Async::Ready(())),
            Ok(WriterState::Pause) => Ok(Async::NotReady),
            Err(err) => Err(err)
        }
    }
}
