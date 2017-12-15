use std::{io, cmp};
use bytes::{Bytes, BytesMut};
use futures::{Async, Poll};
use http2::{Reason, SendStream};
use http2::server::Respond;
use http::{Version, HttpTryFrom, Response};
use http::header::{HeaderValue, CONNECTION, TRANSFER_ENCODING, DATE};

use helpers;
use body::Body;
use helpers::SharedBytes;
use encoding::PayloadEncoder;
use httprequest::HttpMessage;
use httpresponse::HttpResponse;
use h1writer::{Writer, WriterState};

const CHUNK_SIZE: usize = 16_384;
const MAX_WRITE_BUFFER_SIZE: usize = 65_536; // max buffer size 64k

bitflags! {
    struct Flags: u8 {
        const STARTED = 0b0000_0001;
        const DISCONNECTED = 0b0000_0010;
        const EOF = 0b0000_0100;
    }
}

pub(crate) struct H2Writer {
    respond: Respond<Bytes>,
    stream: Option<SendStream<Bytes>>,
    encoder: PayloadEncoder,
    flags: Flags,
    written: u64,
    buffer: SharedBytes,
}

impl H2Writer {

    pub fn new(respond: Respond<Bytes>, buf: SharedBytes) -> H2Writer {
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

    fn write_to_stream(&mut self) -> Result<WriterState, io::Error> {
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
                            return Ok(WriterState::Done)
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

    fn start(&mut self, req: &mut HttpMessage, msg: &mut HttpResponse)
             -> Result<WriterState, io::Error>
    {
        trace!("Prepare response with status: {:?}", msg.status());

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
            helpers::date(&mut bytes);
            msg.headers_mut().insert(DATE, HeaderValue::try_from(&bytes[..]).unwrap());
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

        if msg.body().is_binary() {
            if let Body::Binary(bytes) = msg.replace_body(Body::Empty) {
                self.flags.insert(Flags::EOF);
                self.encoder.write(bytes.as_ref())?;
                if let Some(ref mut stream) = self.stream {
                    stream.reserve_capacity(cmp::min(self.encoder.len(), CHUNK_SIZE));
                }
                return Ok(WriterState::Done)
            }
        }

        Ok(WriterState::Done)
    }

    fn write(&mut self, payload: &[u8]) -> Result<WriterState, io::Error> {
        if !self.flags.contains(Flags::DISCONNECTED) {
            if self.flags.contains(Flags::STARTED) {
                // TODO: add warning, write after EOF
                self.encoder.write(payload)?;
            } else {
                // might be response for EXCEPT
                self.encoder.get_mut().extend_from_slice(payload)
            }
        }

        if self.encoder.len() > MAX_WRITE_BUFFER_SIZE {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    fn write_eof(&mut self) -> Result<WriterState, io::Error> {
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

    fn poll_completed(&mut self) -> Poll<(), io::Error> {
        match self.write_to_stream() {
            Ok(WriterState::Done) => Ok(Async::Ready(())),
            Ok(WriterState::Pause) => Ok(Async::NotReady),
            Err(err) => Err(err)
        }
    }
}
