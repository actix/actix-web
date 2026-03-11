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

    /// Decodes one complete logical line using the request's configured encoding.
    ///
    /// Callers are expected to pass only the bytes that belong to the line being yielded,
    /// whether they came from the internal buffer, the current payload chunk, or both.
    fn decode(encoding: &'static Encoding, bytes: &[u8]) -> Result<String, ReadlinesError> {
        if encoding == UTF_8 {
            str::from_utf8(bytes)
                .map_err(|_| ReadlinesError::EncodingError)
                .map(str::to_owned)
        } else {
            encoding
                .decode_without_bom_handling_and_without_replacement(bytes)
                .map(Cow::into_owned)
                .ok_or(ReadlinesError::EncodingError)
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
                let line = Self::decode(this.encoding, &this.buf.split_to(ind + 1))?;
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
                    if this.buf.len() + ind + 1 > this.limit {
                        return Poll::Ready(Some(Err(ReadlinesError::LimitOverflow)));
                    }

                    this.buf.extend_from_slice(&bytes.split_to(ind + 1));
                    let line = Self::decode(this.encoding, &this.buf)?;
                    this.buf.clear();

                    // buffer bytes following the returned line
                    this.buf.extend_from_slice(&bytes);
                    this.checked_buff = this.buf.is_empty();
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
                let line = Self::decode(this.encoding, &this.buf)?;
                this.buf.clear();
                Poll::Ready(Some(Ok(line)))
            }

            Some(Err(err)) => Poll::Ready(Some(Err(ReadlinesError::from(err)))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use actix_http::{h1, Request};
    use futures_util::{task::noop_waker_ref, StreamExt as _};

    use super::*;
    use crate::{error::ReadlinesError, test::TestRequest};

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

    #[test]
    fn test_readlines_limit_across_chunks() {
        let (mut sender, payload) = h1::Payload::create(false);
        let payload: actix_http::Payload = payload.into();
        let mut req = Request::with_payload(payload);
        let mut stream = Readlines::new(&mut req).limit(10);
        let mut cx = Context::from_waker(noop_waker_ref());

        sender.feed_data(Bytes::from_static(b"AAAAAAAAAA"));
        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Pending
        ));

        sender.feed_data(Bytes::from_static(b"A\n"));
        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(Some(Err(ReadlinesError::LimitOverflow)))
        ));
    }

    #[test]
    fn test_readlines_returns_full_line_across_chunks() {
        let (mut sender, payload) = h1::Payload::create(false);
        let payload: actix_http::Payload = payload.into();
        let mut req = Request::with_payload(payload);
        let mut stream = Readlines::new(&mut req);
        let mut cx = Context::from_waker(noop_waker_ref());

        sender.feed_data(Bytes::from_static(b"hello "));
        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Pending
        ));

        sender.feed_data(Bytes::from_static(b"world\nnext"));
        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(Some(Ok(ref line))) if line == "hello world\n"
        ));
    }
}
