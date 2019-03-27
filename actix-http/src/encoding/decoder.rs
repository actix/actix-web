use std::io::{self, Write};

use bytes::Bytes;
use futures::{Async, Poll, Stream};

#[cfg(feature = "brotli")]
use brotli2::write::BrotliDecoder;
#[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
use flate2::write::{GzDecoder, ZlibDecoder};

use super::Writer;
use crate::error::PayloadError;
use crate::http::header::{ContentEncoding, HeaderMap, CONTENT_ENCODING};

pub struct Decoder<T> {
    stream: T,
    decoder: Option<ContentDecoder>,
}

impl<T> Decoder<T>
where
    T: Stream<Item = Bytes, Error = PayloadError>,
{
    pub fn new(stream: T, encoding: ContentEncoding) -> Self {
        let decoder = match encoding {
            #[cfg(feature = "brotli")]
            ContentEncoding::Br => Some(ContentDecoder::Br(Box::new(
                BrotliDecoder::new(Writer::new()),
            ))),
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoding::Deflate => Some(ContentDecoder::Deflate(Box::new(
                ZlibDecoder::new(Writer::new()),
            ))),
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoding::Gzip => Some(ContentDecoder::Gzip(Box::new(
                GzDecoder::new(Writer::new()),
            ))),
            _ => None,
        };
        Decoder { stream, decoder }
    }

    pub fn from_headers(headers: &HeaderMap, stream: T) -> Self {
        // check content-encoding
        let encoding = if let Some(enc) = headers.get(CONTENT_ENCODING) {
            if let Ok(enc) = enc.to_str() {
                ContentEncoding::from(enc)
            } else {
                ContentEncoding::Identity
            }
        } else {
            ContentEncoding::Identity
        };

        Self::new(stream, encoding)
    }
}

impl<T> Stream for Decoder<T>
where
    T: Stream<Item = Bytes, Error = PayloadError>,
{
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        loop {
            match self.stream.poll()? {
                Async::Ready(Some(chunk)) => {
                    if let Some(ref mut decoder) = self.decoder {
                        match decoder.feed_data(chunk) {
                            Ok(Some(chunk)) => return Ok(Async::Ready(Some(chunk))),
                            Ok(None) => continue,
                            Err(e) => return Err(e.into()),
                        }
                    } else {
                        break;
                    }
                }
                Async::Ready(None) => {
                    return if let Some(mut decoder) = self.decoder.take() {
                        match decoder.feed_eof() {
                            Ok(chunk) => Ok(Async::Ready(chunk)),
                            Err(e) => Err(e.into()),
                        }
                    } else {
                        Ok(Async::Ready(None))
                    };
                }
                Async::NotReady => break,
            }
        }
        Ok(Async::NotReady)
    }
}

enum ContentDecoder {
    #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
    Deflate(Box<ZlibDecoder<Writer>>),
    #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
    Gzip(Box<GzDecoder<Writer>>),
    #[cfg(feature = "brotli")]
    Br(Box<BrotliDecoder<Writer>>),
}

impl ContentDecoder {
    #[allow(unreachable_patterns)]
    fn feed_eof(&mut self) -> io::Result<Option<Bytes>> {
        match self {
            #[cfg(feature = "brotli")]
            ContentDecoder::Br(ref mut decoder) => match decoder.finish() {
                Ok(mut writer) => {
                    let b = writer.take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentDecoder::Gzip(ref mut decoder) => match decoder.try_finish() {
                Ok(_) => {
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentDecoder::Deflate(ref mut decoder) => match decoder.try_finish() {
                Ok(_) => {
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            _ => Ok(None),
        }
    }

    #[allow(unreachable_patterns)]
    fn feed_data(&mut self, data: Bytes) -> io::Result<Option<Bytes>> {
        match self {
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentDecoder::Br(ref mut decoder) => match decoder.write_all(&data) {
                Ok(_) => {
                    decoder.flush()?;
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentDecoder::Gzip(ref mut decoder) => match decoder.write_all(&data) {
                Ok(_) => {
                    decoder.flush()?;
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentDecoder::Deflate(ref mut decoder) => match decoder.write_all(&data) {
                Ok(_) => {
                    decoder.flush()?;
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            _ => Ok(Some(data)),
        }
    }
}
