use std::{io, cmp};
use bytes::Bytes;
use futures::{Async, Poll};
use http2::{Reason, SendStream};
use http2::server::Respond;
use http::{Version, HttpTryFrom, Response};
use http::header::{HeaderValue, CONNECTION, CONTENT_TYPE, TRANSFER_ENCODING, DATE};

use date;
use body::Body;
use encoding::PayloadEncoder;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use h1writer::{Writer, WriterState};

const CHUNK_SIZE: usize = 16_384;
const MAX_WRITE_BUFFER_SIZE: usize = 65_536; // max buffer size 64k


pub(crate) struct H2Writer {
    respond: Respond<Bytes>,
    stream: Option<SendStream<Bytes>>,
    started: bool,
    encoder: PayloadEncoder,
    disconnected: bool,
    eof: bool,
}

impl H2Writer {

    pub fn new(respond: Respond<Bytes>) -> H2Writer {
        H2Writer {
            respond: respond,
            stream: None,
            started: false,
            encoder: PayloadEncoder::default(),
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
            let buffer = self.encoder.get_mut();

            if buffer.is_empty() {
                if self.eof {
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
                        let eof = buffer.is_empty() && self.eof;

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

    fn start(&mut self, req: &mut HttpRequest, msg: &mut HttpResponse)
             -> Result<WriterState, io::Error>
    {
        trace!("Prepare message status={:?}", msg);

        // prepare response
        self.started = true;
        self.encoder = PayloadEncoder::new(req, msg);
        self.eof = if let Body::Empty = *msg.body() { true } else { false };

        // http2 specific
        msg.headers.remove(CONNECTION);
        msg.headers.remove(TRANSFER_ENCODING);

        // using http::h1::date is quite a lot faster than generating
        // a unique Date header each time like req/s goes up about 10%
        if !msg.headers.contains_key(DATE) {
            let mut bytes = [0u8; 29];
            date::extend(&mut bytes[..]);
            msg.headers.insert(DATE, HeaderValue::try_from(&bytes[..]).unwrap());
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
            Ok(stream) =>
                self.stream = Some(stream),
            Err(_) =>
                return Err(io::Error::new(io::ErrorKind::Other, "err")),
        }

        if msg.body().is_binary() {
            if let Body::Binary(bytes) = msg.replace_body(Body::Empty) {
                self.eof = true;
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
        if !self.disconnected {
            if self.started {
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

        self.eof = true;
        if !self.encoder.is_eof() {
            Err(io::Error::new(io::ErrorKind::Other,
                               "Last payload item, but eof is not reached"))
        } else if self.encoder.len() > MAX_WRITE_BUFFER_SIZE {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
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
