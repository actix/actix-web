#![cfg_attr(feature = "cargo-clippy", allow(redundant_field_names))]

use std::cell::RefCell;
use std::fmt::Write as FmtWrite;
use std::io::{self, Write};

#[cfg(feature = "brotli")]
use brotli2::write::BrotliEncoder;
use bytes::{BufMut, BytesMut};
#[cfg(feature = "flate2")]
use flate2::write::{DeflateEncoder, GzEncoder};
#[cfg(feature = "flate2")]
use flate2::Compression;
use futures::{Async, Poll};
use http::header::{HeaderValue, CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, DATE,
                   TRANSFER_ENCODING};
use http::{HttpTryFrom, Version};
use time::{self, Duration};
use tokio_io::AsyncWrite;

use body::{Binary, Body};
use header::ContentEncoding;
use server::encoding::{ContentEncoder, TransferEncoding};
use server::shared::SharedBytes;
use server::WriterState;

use client::ClientRequest;

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
    buffer_capacity: usize,
    encoder: ContentEncoder,
}

impl HttpClientWriter {
    pub fn new(buffer: SharedBytes) -> HttpClientWriter {
        let encoder = ContentEncoder::Identity(TransferEncoding::eof(buffer.clone()));
        HttpClientWriter {
            flags: Flags::empty(),
            written: 0,
            headers_size: 0,
            buffer_capacity: 0,
            buffer,
            encoder,
        }
    }

    pub fn disconnected(&mut self) {
        self.buffer.take();
    }

    pub fn is_completed(&self) -> bool {
        self.buffer.is_empty()
    }

    // pub fn keepalive(&self) -> bool {
    // self.flags.contains(Flags::KEEPALIVE) &&
    // !self.flags.contains(Flags::UPGRADE) }

    fn write_to_stream<T: AsyncWrite>(
        &mut self, stream: &mut T,
    ) -> io::Result<WriterState> {
        while !self.buffer.is_empty() {
            match stream.write(self.buffer.as_ref()) {
                Ok(0) => {
                    self.disconnected();
                    return Ok(WriterState::Done);
                }
                Ok(n) => {
                    let _ = self.buffer.split_to(n);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if self.buffer.len() > self.buffer_capacity {
                        return Ok(WriterState::Pause);
                    } else {
                        return Ok(WriterState::Done);
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

        if msg.upgrade() {
            self.flags.insert(Flags::UPGRADE);
        }

        // render message
        {
            // status line
            writeln!(
                self.buffer,
                "{} {} {:?}\r",
                msg.method(),
                msg.uri()
                    .path_and_query()
                    .map(|u| u.as_str())
                    .unwrap_or("/"),
                msg.version()
            )?;

            // write headers
            let mut buffer = self.buffer.get_mut();
            if let Body::Binary(ref bytes) = *msg.body() {
                buffer.reserve(msg.headers().len() * AVERAGE_HEADER_SIZE + bytes.len());
            } else {
                buffer.reserve(msg.headers().len() * AVERAGE_HEADER_SIZE);
            }

            for (key, value) in msg.headers() {
                let v = value.as_ref();
                let k = key.as_str().as_bytes();
                buffer.reserve(k.len() + v.len() + 4);
                buffer.put_slice(k);
                buffer.put_slice(b": ");
                buffer.put_slice(v);
                buffer.put_slice(b"\r\n");
            }

            // set date header
            if !msg.headers().contains_key(DATE) {
                buffer.extend_from_slice(b"date: ");
                set_date(&mut buffer);
                buffer.extend_from_slice(b"\r\n\r\n");
            } else {
                buffer.extend_from_slice(b"\r\n");
            }
            self.headers_size = buffer.len() as u32;

            if msg.body().is_binary() {
                if let Body::Binary(bytes) = msg.replace_body(Body::Empty) {
                    self.written += bytes.len() as u64;
                    self.encoder.write(bytes)?;
                }
            } else {
                self.buffer_capacity = msg.write_buffer_capacity();
            }
        }
        Ok(())
    }

    pub fn write(&mut self, payload: Binary) -> io::Result<WriterState> {
        self.written += payload.len() as u64;
        if !self.flags.contains(Flags::DISCONNECTED) {
            if self.flags.contains(Flags::UPGRADE) {
                self.buffer.extend(payload);
            } else {
                self.encoder.write(payload)?;
            }
        }

        if self.buffer.len() > self.buffer_capacity {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    pub fn write_eof(&mut self) -> io::Result<()> {
        self.encoder.write_eof()?;

        if self.encoder.is_eof() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "Last payload item, but eof is not reached",
            ))
        }
    }

    #[inline]
    pub fn poll_completed<T: AsyncWrite>(
        &mut self, stream: &mut T, shutdown: bool,
    ) -> Poll<(), io::Error> {
        match self.write_to_stream(stream) {
            Ok(WriterState::Done) => {
                if shutdown {
                    stream.shutdown()
                } else {
                    Ok(Async::Ready(()))
                }
            }
            Ok(WriterState::Pause) => Ok(Async::NotReady),
            Err(err) => Err(err),
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
        }
        Body::Binary(ref mut bytes) => {
            if encoding.is_compression() {
                let tmp = SharedBytes::default();
                let transfer = TransferEncoding::eof(tmp.clone());
                let mut enc = match encoding {
                    #[cfg(feature = "flate2")]
                    ContentEncoding::Deflate => ContentEncoder::Deflate(
                        DeflateEncoder::new(transfer, Compression::default()),
                    ),
                    #[cfg(feature = "flate2")]
                    ContentEncoding::Gzip => ContentEncoder::Gzip(GzEncoder::new(
                        transfer,
                        Compression::default(),
                    )),
                    #[cfg(feature = "brotli")]
                    ContentEncoding::Br => {
                        ContentEncoder::Br(BrotliEncoder::new(transfer, 5))
                    }
                    ContentEncoding::Identity => ContentEncoder::Identity(transfer),
                    ContentEncoding::Auto => unreachable!(),
                };
                // TODO return error!
                let _ = enc.write(bytes.clone());
                let _ = enc.write_eof();
                *bytes = Binary::from(tmp.take());

                req.headers_mut().insert(
                    CONTENT_ENCODING,
                    HeaderValue::from_static(encoding.as_str()),
                );
                encoding = ContentEncoding::Identity;
            }
            let mut b = BytesMut::new();
            let _ = write!(b, "{}", bytes.len());
            req.headers_mut().insert(
                CONTENT_LENGTH,
                HeaderValue::try_from(b.freeze()).unwrap(),
            );
            TransferEncoding::eof(buf)
        }
        Body::Streaming(_) | Body::Actor(_) => {
            if req.upgrade() {
                if version == Version::HTTP_2 {
                    error!("Connection upgrade is forbidden for HTTP/2");
                } else {
                    req.headers_mut()
                        .insert(CONNECTION, HeaderValue::from_static("upgrade"));
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

    if encoding.is_compression() {
        req.headers_mut().insert(
            CONTENT_ENCODING,
            HeaderValue::from_static(encoding.as_str()),
        );
    }

    req.replace_body(body);
    match encoding {
        #[cfg(feature = "flate2")]
        ContentEncoding::Deflate => ContentEncoder::Deflate(DeflateEncoder::new(
            transfer,
            Compression::default(),
        )),
        #[cfg(feature = "flate2")]
        ContentEncoding::Gzip => {
            ContentEncoder::Gzip(GzEncoder::new(transfer, Compression::default()))
        }
        #[cfg(feature = "brotli")]
        ContentEncoding::Br => ContentEncoder::Br(BrotliEncoder::new(transfer, 5)),
        ContentEncoding::Identity | ContentEncoding::Auto => {
            ContentEncoder::Identity(transfer)
        }
    }
}

fn streaming_encoding(
    buf: SharedBytes, version: Version, req: &mut ClientRequest,
) -> TransferEncoding {
    if req.chunked() {
        // Enable transfer encoding
        req.headers_mut().remove(CONTENT_LENGTH);
        if version == Version::HTTP_2 {
            req.headers_mut().remove(TRANSFER_ENCODING);
            TransferEncoding::eof(buf)
        } else {
            req.headers_mut()
                .insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
            TransferEncoding::chunked(buf)
        }
    } else {
        // if Content-Length is specified, then use it as length hint
        let (len, chunked) = if let Some(len) = req.headers().get(CONTENT_LENGTH) {
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
                    req.headers_mut()
                        .insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
                    TransferEncoding::chunked(buf)
                }
                _ => {
                    req.headers_mut().remove(TRANSFER_ENCODING);
                    TransferEncoding::eof(buf)
                }
            }
        }
    }
}

// "Sun, 06 Nov 1994 08:49:37 GMT".len()
pub const DATE_VALUE_LENGTH: usize = 29;

fn set_date(dst: &mut BytesMut) {
    CACHED.with(|cache| {
        let mut cache = cache.borrow_mut();
        let now = time::get_time();
        if now > cache.next_update {
            cache.update(now);
        }
        dst.extend_from_slice(cache.buffer());
    })
}

struct CachedDate {
    bytes: [u8; DATE_VALUE_LENGTH],
    next_update: time::Timespec,
}

thread_local!(static CACHED: RefCell<CachedDate> = RefCell::new(CachedDate {
    bytes: [0; DATE_VALUE_LENGTH],
    next_update: time::Timespec::new(0, 0),
}));

impl CachedDate {
    fn buffer(&self) -> &[u8] {
        &self.bytes[..]
    }

    fn update(&mut self, now: time::Timespec) {
        write!(&mut self.bytes[..], "{}", time::at_utc(now).rfc822()).unwrap();
        self.next_update = now + Duration::seconds(1);
        self.next_update.nsec = 0;
    }
}
