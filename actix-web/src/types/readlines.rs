//! For request line reader documentation, see [`Readlines`].

use std::{
    borrow::Cow,
    pin::Pin,
    str,
    task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};
use encoding_rs::{Encoding, UTF_8};
use futures_core::{ready, stream::Stream};

use crate::{
    dev::Payload,
    error::{PayloadError, ReadlinesError},
    HttpMessage,
};

/// Stream that reads request line by line.
pub struct Readlines<T: HttpMessage> {
    stream: Payload<T::Stream>,
    buf: BytesMut,
    limit: usize,
    checked_buff: bool,
    encoding: &'static Encoding,
    err: Option<ReadlinesError>,
}

impl<T> Readlines<T>
where
    T: HttpMessage,
    T::Stream: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    /// Create a new stream to read request line by line.
    pub fn new(req: &mut T) -> Self {
        let encoding = match req.encoding() {
            Ok(enc) => enc,
            Err(err) => return Self::err(err.into()),
        };

        Readlines {
            stream: req.take_payload(),
            buf: BytesMut::with_capacity(262_144),
            limit: 262_144,
            checked_buff: true,
            err: None,
            encoding,
        }
    }

    /// Set maximum accepted payload size. The default limit is 256kB.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    fn err(err: ReadlinesError) -> Self {
        Readlines {
            stream: Payload::None,
            buf: BytesMut::new(),
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
    T::Stream: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    type Item = Result<String, ReadlinesError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if let Some(err) = this.err.take() {
            return Poll::Ready(Some(Err(err)));
        }

        // check if there is a newline in the buffer
        if !this.checked_buff {
            let mut found: Option<usize> = None;
            for (ind, b) in this.buf.iter().enumerate() {
                if *b == b'\n' {
                    found = Some(ind);
                    break;
                }
            }
            if let Some(ind) = found {
                // check if line is longer than limit
                if ind + 1 > this.limit {
                    return Poll::Ready(Some(Err(ReadlinesError::LimitOverflow)));
                }
                let line = if this.encoding == UTF_8 {
                    str::from_utf8(&this.buf.split_to(ind + 1))
                        .map_err(|_| ReadlinesError::EncodingError)?
                        .to_owned()
                } else {
                    this.encoding
                        .decode_without_bom_handling_and_without_replacement(
                            &this.buf.split_to(ind + 1),
                        )
                        .map(Cow::into_owned)
                        .ok_or(ReadlinesError::EncodingError)?
                };
                return Poll::Ready(Some(Ok(line)));
            }
            this.checked_buff = true;
        }

        // poll req for more bytes
        match ready!(Pin::new(&mut this.stream).poll_next(cx)) {
            Some(Ok(mut bytes)) => {
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
                    if ind + 1 > this.limit {
                        return Poll::Ready(Some(Err(ReadlinesError::LimitOverflow)));
                    }
                    let line = if this.encoding == UTF_8 {
                        str::from_utf8(&bytes.split_to(ind + 1))
                            .map_err(|_| ReadlinesError::EncodingError)?
                            .to_owned()
                    } else {
                        this.encoding
                            .decode_without_bom_handling_and_without_replacement(
                                &bytes.split_to(ind + 1),
                            )
                            .map(Cow::into_owned)
                            .ok_or(ReadlinesError::EncodingError)?
                    };
                    // extend buffer with rest of the bytes;
                    this.buf.extend_from_slice(&bytes);
                    this.checked_buff = false;
                    return Poll::Ready(Some(Ok(line)));
                }
                this.buf.extend_from_slice(&bytes);
                Poll::Pending
            }

            None => {
                if this.buf.is_empty() {
                    return Poll::Ready(None);
                }
                if this.buf.len() > this.limit {
                    return Poll::Ready(Some(Err(ReadlinesError::LimitOverflow)));
                }
                let line = if this.encoding == UTF_8 {
                    str::from_utf8(&this.buf)
                        .map_err(|_| ReadlinesError::EncodingError)?
                        .to_owned()
                } else {
                    this.encoding
                        .decode_without_bom_handling_and_without_replacement(&this.buf)
                        .map(Cow::into_owned)
                        .ok_or(ReadlinesError::EncodingError)?
                };
                this.buf.clear();
                Poll::Ready(Some(Ok(line)))
            }

            Some(Err(err)) => Poll::Ready(Some(Err(ReadlinesError::from(err)))),
        }
    }
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt as _;

    use super::*;
    use crate::test::TestRequest;

    #[actix_rt::test]
    async fn test_readlines() {
        let mut req = TestRequest::default()
            .set_payload(Bytes::from_static(
                b"Lorem Ipsum is simply dummy text of the printing and typesetting\n\
                  industry. Lorem Ipsum has been the industry's standard dummy\n\
                  Contrary to popular belief, Lorem Ipsum is not simply random text.",
            ))
            .to_request();

        let mut stream = Readlines::new(&mut req);
        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            "Lorem Ipsum is simply dummy text of the printing and typesetting\n"
        );

        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            "industry. Lorem Ipsum has been the industry's standard dummy\n"
        );

        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            "Contrary to popular belief, Lorem Ipsum is not simply random text."
        );
    }
}
