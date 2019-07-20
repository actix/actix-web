use std::borrow::Cow;
use std::str;

use bytes::{Bytes, BytesMut};
use encoding_rs::{Encoding, UTF_8};
use futures::{Async, Poll, Stream};

use crate::dev::Payload;
use crate::error::{PayloadError, ReadlinesError};
use crate::HttpMessage;

/// Stream to read request line by line.
pub struct Readlines<T: HttpMessage> {
    stream: Payload<T::Stream>,
    buff: BytesMut,
    limit: usize,
    checked_buff: bool,
    encoding: &'static Encoding,
    err: Option<ReadlinesError>,
}

impl<T> Readlines<T>
where
    T: HttpMessage,
    T::Stream: Stream<Item = Bytes, Error = PayloadError>,
{
    /// Create a new stream to read request line by line.
    pub fn new(req: &mut T) -> Self {
        let encoding = match req.encoding() {
            Ok(enc) => enc,
            Err(err) => return Self::err(err.into()),
        };

        Readlines {
            stream: req.take_payload(),
            buff: BytesMut::with_capacity(262_144),
            limit: 262_144,
            checked_buff: true,
            err: None,
            encoding,
        }
    }

    /// Change max line size. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    fn err(err: ReadlinesError) -> Self {
        Readlines {
            stream: Payload::None,
            buff: BytesMut::new(),
            limit: 262_144,
            checked_buff: true,
            encoding: UTF_8,
            err: Some(err),
        }
    }
}

impl<T> Stream for Readlines<T>
where
    T: HttpMessage,
    T::Stream: Stream<Item = Bytes, Error = PayloadError>,
{
    type Item = String;
    type Error = ReadlinesError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some(err) = self.err.take() {
            return Err(err);
        }

        // check if there is a newline in the buffer
        if !self.checked_buff {
            let mut found: Option<usize> = None;
            for (ind, b) in self.buff.iter().enumerate() {
                if *b == b'\n' {
                    found = Some(ind);
                    break;
                }
            }
            if let Some(ind) = found {
                // check if line is longer than limit
                if ind + 1 > self.limit {
                    return Err(ReadlinesError::LimitOverflow);
                }
                let line = if self.encoding == UTF_8 {
                    str::from_utf8(&self.buff.split_to(ind + 1))
                        .map_err(|_| ReadlinesError::EncodingError)?
                        .to_owned()
                } else {
                    self.encoding
                        .decode_without_bom_handling_and_without_replacement(
                            &self.buff.split_to(ind + 1),
                        )
                        .map(Cow::into_owned)
                        .ok_or(ReadlinesError::EncodingError)?
                };
                return Ok(Async::Ready(Some(line)));
            }
            self.checked_buff = true;
        }
        // poll req for more bytes
        match self.stream.poll() {
            Ok(Async::Ready(Some(mut bytes))) => {
                // check if there is a newline in bytes
                let mut found: Option<usize> = None;
                for (ind, b) in bytes.iter().enumerate() {
                    if *b == b'\n' {
                        found = Some(ind);
                        break;
                    }
                }
                if let Some(ind) = found {
                    // check if line is longer than limit
                    if ind + 1 > self.limit {
                        return Err(ReadlinesError::LimitOverflow);
                    }
                    let line = if self.encoding == UTF_8 {
                        str::from_utf8(&bytes.split_to(ind + 1))
                            .map_err(|_| ReadlinesError::EncodingError)?
                            .to_owned()
                    } else {
                        self.encoding
                            .decode_without_bom_handling_and_without_replacement(
                                &bytes.split_to(ind + 1),
                            )
                            .map(Cow::into_owned)
                            .ok_or(ReadlinesError::EncodingError)?
                    };
                    // extend buffer with rest of the bytes;
                    self.buff.extend_from_slice(&bytes);
                    self.checked_buff = false;
                    return Ok(Async::Ready(Some(line)));
                }
                self.buff.extend_from_slice(&bytes);
                Ok(Async::NotReady)
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(None)) => {
                if self.buff.is_empty() {
                    return Ok(Async::Ready(None));
                }
                if self.buff.len() > self.limit {
                    return Err(ReadlinesError::LimitOverflow);
                }
                let line = if self.encoding == UTF_8 {
                    str::from_utf8(&self.buff)
                        .map_err(|_| ReadlinesError::EncodingError)?
                        .to_owned()
                } else {
                    self.encoding
                        .decode_without_bom_handling_and_without_replacement(&self.buff)
                        .map(Cow::into_owned)
                        .ok_or(ReadlinesError::EncodingError)?
                };
                self.buff.clear();
                Ok(Async::Ready(Some(line)))
            }
            Err(e) => Err(ReadlinesError::from(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::{block_on, TestRequest};

    #[test]
    fn test_readlines() {
        let mut req = TestRequest::default()
            .set_payload(Bytes::from_static(
                b"Lorem Ipsum is simply dummy text of the printing and typesetting\n\
                  industry. Lorem Ipsum has been the industry's standard dummy\n\
                  Contrary to popular belief, Lorem Ipsum is not simply random text.",
            ))
            .to_request();
        let stream = match block_on(Readlines::new(&mut req).into_future()) {
            Ok((Some(s), stream)) => {
                assert_eq!(
                    s,
                    "Lorem Ipsum is simply dummy text of the printing and typesetting\n"
                );
                stream
            }
            _ => unreachable!("error"),
        };

        let stream = match block_on(stream.into_future()) {
            Ok((Some(s), stream)) => {
                assert_eq!(
                    s,
                    "industry. Lorem Ipsum has been the industry's standard dummy\n"
                );
                stream
            }
            _ => unreachable!("error"),
        };

        match block_on(stream.into_future()) {
            Ok((Some(s), _)) => {
                assert_eq!(
                    s,
                    "Contrary to popular belief, Lorem Ipsum is not simply random text."
                );
            }
            _ => unreachable!("error"),
        }
    }
}
