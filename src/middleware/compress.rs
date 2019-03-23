use std::io::Write;
use std::marker::PhantomData;
use std::str::FromStr;
use std::{cmp, fmt, io};

use actix_http::body::{Body, BodyLength, MessageBody, ResponseBody};
use actix_http::http::header::{
    ContentEncoding, HeaderValue, ACCEPT_ENCODING, CONTENT_ENCODING,
};
use actix_http::http::{HttpTryFrom, StatusCode};
use actix_http::{Error, Head, ResponseHead};
use actix_service::{Service, Transform};
use bytes::{Bytes, BytesMut};
use futures::future::{ok, FutureResult};
use futures::{Async, Future, Poll};
use log::trace;

#[cfg(feature = "brotli")]
use brotli2::write::BrotliEncoder;
#[cfg(feature = "flate2")]
use flate2::write::{GzEncoder, ZlibEncoder};

use crate::service::{ServiceRequest, ServiceResponse};

#[derive(Debug, Clone)]
pub struct Compress(ContentEncoding);

impl Compress {
    pub fn new(encoding: ContentEncoding) -> Self {
        Compress(encoding)
    }
}

impl Default for Compress {
    fn default() -> Self {
        Compress::new(ContentEncoding::Auto)
    }
}

impl<S, P, B> Transform<S> for Compress
where
    P: 'static,
    B: MessageBody,
    S: Service<Request = ServiceRequest<P>, Response = ServiceResponse<B>>,
    S::Future: 'static,
{
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse<Encoder<B>>;
    type Error = S::Error;
    type InitError = ();
    type Transform = CompressMiddleware<S>;
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(CompressMiddleware {
            service,
            encoding: self.0,
        })
    }
}

pub struct CompressMiddleware<S> {
    service: S,
    encoding: ContentEncoding,
}

impl<S, P, B> Service for CompressMiddleware<S>
where
    P: 'static,
    B: MessageBody,
    S: Service<Request = ServiceRequest<P>, Response = ServiceResponse<B>>,
    S::Future: 'static,
{
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse<Encoder<B>>;
    type Error = S::Error;
    type Future = CompressResponse<S, P, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: ServiceRequest<P>) -> Self::Future {
        // negotiate content-encoding
        let encoding = if let Some(val) = req.headers.get(ACCEPT_ENCODING) {
            if let Ok(enc) = val.to_str() {
                AcceptEncoding::parse(enc, self.encoding)
            } else {
                ContentEncoding::Identity
            }
        } else {
            ContentEncoding::Identity
        };

        CompressResponse {
            encoding,
            fut: self.service.call(req),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct CompressResponse<S, P, B>
where
    P: 'static,
    B: MessageBody,
    S: Service,
    S::Future: 'static,
{
    fut: S::Future,
    encoding: ContentEncoding,
    _t: PhantomData<(P, B)>,
}

impl<S, P, B> Future for CompressResponse<S, P, B>
where
    P: 'static,
    B: MessageBody,
    S: Service<Request = ServiceRequest<P>, Response = ServiceResponse<B>>,
    S::Future: 'static,
{
    type Item = ServiceResponse<Encoder<B>>;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let resp = futures::try_ready!(self.fut.poll());

        Ok(Async::Ready(resp.map_body(move |head, body| {
            Encoder::body(self.encoding, head, body)
        })))
    }
}

enum EncoderBody<B> {
    Body(B),
    Other(Box<dyn MessageBody>),
}

pub struct Encoder<B> {
    body: EncoderBody<B>,
    encoder: Option<ContentEncoder>,
}

impl<B: MessageBody> MessageBody for Encoder<B> {
    fn length(&self) -> BodyLength {
        if self.encoder.is_none() {
            match self.body {
                EncoderBody::Body(ref b) => b.length(),
                EncoderBody::Other(ref b) => b.length(),
            }
        } else {
            BodyLength::Stream
        }
    }

    fn poll_next(&mut self) -> Poll<Option<Bytes>, Error> {
        loop {
            let result = match self.body {
                EncoderBody::Body(ref mut b) => b.poll_next()?,
                EncoderBody::Other(ref mut b) => b.poll_next()?,
            };
            match result {
                Async::NotReady => return Ok(Async::NotReady),
                Async::Ready(Some(chunk)) => {
                    if let Some(ref mut encoder) = self.encoder {
                        if encoder.write(&chunk)? {
                            return Ok(Async::Ready(Some(encoder.take())));
                        }
                    } else {
                        return Ok(Async::Ready(Some(chunk)));
                    }
                }
                Async::Ready(None) => {
                    if let Some(encoder) = self.encoder.take() {
                        let chunk = encoder.finish()?;
                        if chunk.is_empty() {
                            return Ok(Async::Ready(None));
                        } else {
                            return Ok(Async::Ready(Some(chunk)));
                        }
                    } else {
                        return Ok(Async::Ready(None));
                    }
                }
            }
        }
    }
}

fn update_head(encoding: ContentEncoding, head: &mut ResponseHead) {
    head.headers_mut().insert(
        CONTENT_ENCODING,
        HeaderValue::try_from(Bytes::from_static(encoding.as_str().as_bytes())).unwrap(),
    );
}

impl<B: MessageBody> Encoder<B> {
    fn body(
        encoding: ContentEncoding,
        head: &mut ResponseHead,
        body: ResponseBody<B>,
    ) -> ResponseBody<Encoder<B>> {
        let has_ce = head.headers().contains_key(CONTENT_ENCODING);
        match body {
            ResponseBody::Other(b) => match b {
                Body::None => ResponseBody::Other(Body::None),
                Body::Empty => ResponseBody::Other(Body::Empty),
                Body::Bytes(buf) => {
                    if !(has_ce
                        || encoding == ContentEncoding::Identity
                        || encoding == ContentEncoding::Auto)
                    {
                        let mut enc = ContentEncoder::encoder(encoding).unwrap();

                        // TODO return error!
                        let _ = enc.write(buf.as_ref());
                        let body = enc.finish().unwrap();
                        update_head(encoding, head);
                        ResponseBody::Other(Body::Bytes(body))
                    } else {
                        ResponseBody::Other(Body::Bytes(buf))
                    }
                }
                Body::Message(stream) => {
                    if has_ce || head.status == StatusCode::SWITCHING_PROTOCOLS {
                        ResponseBody::Body(Encoder {
                            body: EncoderBody::Other(stream),
                            encoder: None,
                        })
                    } else {
                        update_head(encoding, head);
                        head.no_chunking = false;
                        ResponseBody::Body(Encoder {
                            body: EncoderBody::Other(stream),
                            encoder: ContentEncoder::encoder(encoding),
                        })
                    }
                }
            },
            ResponseBody::Body(stream) => {
                if has_ce || head.status == StatusCode::SWITCHING_PROTOCOLS {
                    ResponseBody::Body(Encoder {
                        body: EncoderBody::Body(stream),
                        encoder: None,
                    })
                } else {
                    update_head(encoding, head);
                    head.no_chunking = false;
                    ResponseBody::Body(Encoder {
                        body: EncoderBody::Body(stream),
                        encoder: ContentEncoder::encoder(encoding),
                    })
                }
            }
        }
    }
}

pub(crate) struct Writer {
    buf: BytesMut,
}

impl Writer {
    fn new() -> Writer {
        Writer {
            buf: BytesMut::with_capacity(8192),
        }
    }
    fn take(&mut self) -> Bytes {
        self.buf.take().freeze()
    }
}

impl io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) enum ContentEncoder {
    #[cfg(feature = "flate2")]
    Deflate(ZlibEncoder<Writer>),
    #[cfg(feature = "flate2")]
    Gzip(GzEncoder<Writer>),
    #[cfg(feature = "brotli")]
    Br(BrotliEncoder<Writer>),
}

impl fmt::Debug for ContentEncoder {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(_) => writeln!(f, "ContentEncoder(Brotli)"),
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(_) => writeln!(f, "ContentEncoder(Deflate)"),
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(_) => writeln!(f, "ContentEncoder(Gzip)"),
        }
    }
}

impl ContentEncoder {
    fn encoder(encoding: ContentEncoding) -> Option<Self> {
        match encoding {
            #[cfg(feature = "flate2")]
            ContentEncoding::Deflate => Some(ContentEncoder::Deflate(ZlibEncoder::new(
                Writer::new(),
                flate2::Compression::fast(),
            ))),
            #[cfg(feature = "flate2")]
            ContentEncoding::Gzip => Some(ContentEncoder::Gzip(GzEncoder::new(
                Writer::new(),
                flate2::Compression::fast(),
            ))),
            #[cfg(feature = "brotli")]
            ContentEncoding::Br => {
                Some(ContentEncoder::Br(BrotliEncoder::new(Writer::new(), 3)))
            }
            _ => None,
        }
    }

    #[inline]
    pub(crate) fn take(&mut self) -> Bytes {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(ref mut encoder) => encoder.get_mut().take(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(ref mut encoder) => encoder.get_mut().take(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(ref mut encoder) => encoder.get_mut().take(),
        }
    }

    fn finish(self) -> Result<Bytes, io::Error> {
        match self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<bool, io::Error> {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(!encoder.get_ref().buf.is_empty()),
                Err(err) => {
                    trace!("Error decoding br encoding: {}", err);
                    Err(err)
                }
            },
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(!encoder.get_ref().buf.is_empty()),
                Err(err) => {
                    trace!("Error decoding gzip encoding: {}", err);
                    Err(err)
                }
            },
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(!encoder.get_ref().buf.is_empty()),
                Err(err) => {
                    trace!("Error decoding deflate encoding: {}", err);
                    Err(err)
                }
            },
        }
    }
}

struct AcceptEncoding {
    encoding: ContentEncoding,
    quality: f64,
}

impl Eq for AcceptEncoding {}

impl Ord for AcceptEncoding {
    fn cmp(&self, other: &AcceptEncoding) -> cmp::Ordering {
        if self.quality > other.quality {
            cmp::Ordering::Less
        } else if self.quality < other.quality {
            cmp::Ordering::Greater
        } else {
            cmp::Ordering::Equal
        }
    }
}

impl PartialOrd for AcceptEncoding {
    fn partial_cmp(&self, other: &AcceptEncoding) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for AcceptEncoding {
    fn eq(&self, other: &AcceptEncoding) -> bool {
        self.quality == other.quality
    }
}

impl AcceptEncoding {
    fn new(tag: &str) -> Option<AcceptEncoding> {
        let parts: Vec<&str> = tag.split(';').collect();
        let encoding = match parts.len() {
            0 => return None,
            _ => ContentEncoding::from(parts[0]),
        };
        let quality = match parts.len() {
            1 => encoding.quality(),
            _ => match f64::from_str(parts[1]) {
                Ok(q) => q,
                Err(_) => 0.0,
            },
        };
        Some(AcceptEncoding { encoding, quality })
    }

    /// Parse a raw Accept-Encoding header value into an ordered list.
    pub fn parse(raw: &str, encoding: ContentEncoding) -> ContentEncoding {
        let mut encodings: Vec<_> = raw
            .replace(' ', "")
            .split(',')
            .map(|l| AcceptEncoding::new(l))
            .collect();
        encodings.sort();

        for enc in encodings {
            if let Some(enc) = enc {
                if encoding == ContentEncoding::Auto {
                    return enc.encoding;
                } else if encoding == enc.encoding {
                    return encoding;
                }
            }
        }
        ContentEncoding::Identity
    }
}
