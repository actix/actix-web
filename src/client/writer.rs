#![allow(dead_code)]
use std::io;
use std::fmt::Write;
use bytes::{BytesMut, BufMut};
use futures::{Async, Poll};
use tokio_io::AsyncWrite;
use http::{Version, HttpTryFrom};
use http::header::{HeaderValue, CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, TRANSFER_ENCODING};
use flate2::Compression;
use flate2::write::{GzEncoder, DeflateEncoder};
use brotli2::write::BrotliEncoder;

use body::{Body, Binary};
use headers::ContentEncoding;
use server::WriterState;
use server::shared::SharedBytes;
use server::encoding::{ContentEncoder, TransferEncoding};

use client::ClientRequest;


const LOW_WATERMARK: usize = 1024;
const HIGH_WATERMARK: usize = 8 * LOW_WATERMARK;
const AVERAGE_HEADER_SIZE: usize = 30;

bitflags! {
    struct Flags: u8 {
        const STARTED = 0b0000_0001;
        const UPGRADE = 0b0000_0010;
        const KEEPALIVE = 0b0000_0100;
        const DISCONNECTED = 0b0000_1000;
    }
}

pub(crate) struct HttpClientWriter {
    flags: Flags,
    written: u64,
    headers_size: u32,
    buffer: SharedBytes,
    encoder: ContentEncoder,
    low: usize,
    high: usize,
}

impl HttpClientWriter {

    pub fn new(buf: SharedBytes) -> HttpClientWriter {
        let encoder = ContentEncoder::Identity(TransferEncoding::eof(buf.clone()));
        HttpClientWriter {
            flags: Flags::empty(),
            written: 0,
            headers_size: 0,
            buffer: buf,
            encoder: encoder,
            low: LOW_WATERMARK,
            high: HIGH_WATERMARK,
        }
    }

    pub fn disconnected(&mut self) {
        self.buffer.take();
    }

    pub fn keepalive(&self) -> bool {
        self.flags.contains(Flags::KEEPALIVE) && !self.flags.contains(Flags::UPGRADE)
    }

    /// Set write buffer capacity
    pub fn set_buffer_capacity(&mut self, low_watermark: usize, high_watermark: usize) {
        self.low = low_watermark;
        self.high = high_watermark;
    }

    fn write_to_stream<T: AsyncWrite>(&mut self, stream: &mut T) -> io::Result<WriterState> {
        while !self.buffer.is_empty() {
            match stream.write(self.buffer.as_ref()) {
                Ok(0) => {
                    self.disconnected();
                    return Ok(WriterState::Done);
                },
                Ok(n) => {
                    let _ = self.buffer.split_to(n);
                },
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if self.buffer.len() > self.high {
                        return Ok(WriterState::Pause)
                    } else {
                        return Ok(WriterState::Done)
                    }
                }
                Err(err) => return Err(err),
            }
        }
        Ok(WriterState::Done)
    }
}

impl HttpClientWriter {

    pub fn start(&mut self, msg: &mut ClientRequest) -> io::Result<()> {
        // prepare task
        self.flags.insert(Flags::STARTED);
        self.encoder = content_encoder(self.buffer.clone(), msg);

        // render message
        {
            let buffer = self.buffer.get_mut();
            if let Body::Binary(ref bytes) = *msg.body() {
                buffer.reserve(256 + msg.headers().len() * AVERAGE_HEADER_SIZE + bytes.len());
            } else {
                buffer.reserve(256 + msg.headers().len() * AVERAGE_HEADER_SIZE);
            }

            // status line
            let _ = write!(buffer, "{} {} {:?}\r\n",
                           msg.method(), msg.uri().path(), msg.version());

            // write headers
            for (key, value) in msg.headers() {
                let v = value.as_ref();
                let k = key.as_str().as_bytes();
                buffer.reserve(k.len() + v.len() + 4);
                buffer.put_slice(k);
                buffer.put_slice(b": ");
                buffer.put_slice(v);
                buffer.put_slice(b"\r\n");
            }

            // using helpers::date is quite a lot faster
            //if !msg.headers.contains_key(DATE) {
            //    helpers::date(&mut buffer);
            //} else {
                // msg eof
                buffer.extend_from_slice(b"\r\n");
            //}
            self.headers_size = buffer.len() as u32;

            if msg.body().is_binary() {
                if let Body::Binary(bytes) = msg.replace_body(Body::Empty) {
                    self.written += bytes.len() as u64;
                    self.encoder.write(bytes)?;
                }
            }
        }
        Ok(())
    }

    pub fn write(&mut self, payload: &Binary) -> io::Result<WriterState> {
        self.written += payload.len() as u64;
        if !self.flags.contains(Flags::DISCONNECTED) {
            self.buffer.extend_from_slice(payload.as_ref())
        }

        if self.buffer.len() > self.high {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    pub fn write_eof(&mut self) -> io::Result<WriterState> {
        if self.buffer.len() > self.high {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    #[inline]
    pub fn poll_completed<T: AsyncWrite>(&mut self, stream: &mut T, shutdown: bool)
                                         -> Poll<(), io::Error>
    {
        match self.write_to_stream(stream) {
            Ok(WriterState::Done) => {
                if shutdown {
                    stream.shutdown()
                } else {
                    Ok(Async::Ready(()))
                }
            },
            Ok(WriterState::Pause) => Ok(Async::NotReady),
            Err(err) => Err(err)
        }
    }
}


fn content_encoder(buf: SharedBytes, req: &mut ClientRequest) -> ContentEncoder {
    let version = req.version();
    let mut body = req.replace_body(Body::Empty);
    let mut encoding = req.content_encoding();

    let transfer = match body {
        Body::Empty => {
            req.headers_mut().remove(CONTENT_LENGTH);
            TransferEncoding::length(0, buf)
        },
        Body::Binary(ref mut bytes) => {
            if encoding.is_compression() {
                let tmp = SharedBytes::default();
                let transfer = TransferEncoding::eof(tmp.clone());
                let mut enc = match encoding {
                    ContentEncoding::Deflate => ContentEncoder::Deflate(
                        DeflateEncoder::new(transfer, Compression::default())),
                    ContentEncoding::Gzip => ContentEncoder::Gzip(
                        GzEncoder::new(transfer, Compression::default())),
                    ContentEncoding::Br => ContentEncoder::Br(
                        BrotliEncoder::new(transfer, 5)),
                    ContentEncoding::Identity => ContentEncoder::Identity(transfer),
                    ContentEncoding::Auto => unreachable!()
                };
                // TODO return error!
                let _ = enc.write(bytes.clone());
                let _ = enc.write_eof();

                *bytes = Binary::from(tmp.take());
                encoding = ContentEncoding::Identity;
            }
            let mut b = BytesMut::new();
            let _ = write!(b, "{}", bytes.len());
            req.headers_mut().insert(
                CONTENT_LENGTH, HeaderValue::try_from(b.freeze()).unwrap());
            TransferEncoding::eof(buf)
        },
        Body::Streaming(_) | Body::Actor(_) => {
            if req.upgrade() {
                if version == Version::HTTP_2 {
                    error!("Connection upgrade is forbidden for HTTP/2");
                } else {
                    req.headers_mut().insert(CONNECTION, HeaderValue::from_static("upgrade"));
                }
                if encoding != ContentEncoding::Identity {
                    encoding = ContentEncoding::Identity;
                    req.headers_mut().remove(CONTENT_ENCODING);
                }
                TransferEncoding::eof(buf)
            } else {
                streaming_encoding(buf, version, req)
            }
        }
    };

    req.replace_body(body);
    match encoding {
        ContentEncoding::Deflate => ContentEncoder::Deflate(
            DeflateEncoder::new(transfer, Compression::default())),
        ContentEncoding::Gzip => ContentEncoder::Gzip(
            GzEncoder::new(transfer, Compression::default())),
        ContentEncoding::Br => ContentEncoder::Br(
            BrotliEncoder::new(transfer, 5)),
        ContentEncoding::Identity | ContentEncoding::Auto => ContentEncoder::Identity(transfer),
    }
}

fn streaming_encoding(buf: SharedBytes, version: Version, req: &mut ClientRequest)
                      -> TransferEncoding {
    if req.chunked() {
        // Enable transfer encoding
        req.headers_mut().remove(CONTENT_LENGTH);
        if version == Version::HTTP_2 {
            req.headers_mut().remove(TRANSFER_ENCODING);
            TransferEncoding::eof(buf)
        } else {
            req.headers_mut().insert(
                TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
            TransferEncoding::chunked(buf)
        }
    } else {
        // if Content-Length is specified, then use it as length hint
        let (len, chunked) =
            if let Some(len) = req.headers().get(CONTENT_LENGTH) {
                // Content-Length
                if let Ok(s) = len.to_str() {
                    if let Ok(len) = s.parse::<u64>() {
                        (Some(len), false)
                    } else {
                        error!("illegal Content-Length: {:?}", len);
                        (None, false)
                    }
                } else {
                    error!("illegal Content-Length: {:?}", len);
                    (None, false)
                }
            } else {
                (None, true)
            };

        if !chunked {
            if let Some(len) = len {
                TransferEncoding::length(len, buf)
            } else {
                TransferEncoding::eof(buf)
            }
        } else {
            // Enable transfer encoding
            match version {
                Version::HTTP_11 => {
                    req.headers_mut().insert(
                        TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
                    TransferEncoding::chunked(buf)
                },
                _ => {
                    req.headers_mut().remove(TRANSFER_ENCODING);
                    TransferEncoding::eof(buf)
                }
            }
        }
    }
}
