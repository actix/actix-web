use std::{
    future::Future,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::{error::PayloadError, header, HttpMessage};
use bytes::Bytes;
use futures_core::Stream;

use super::{read_body::ReadBody, ResponseTimeout, DEFAULT_BODY_LIMIT};
use crate::ClientResponse;

/// Future that resolves to a complete response body.
pub struct ResponseBody<S> {
    body: Result<ReadBody<S>, Option<PayloadError>>,
    length: Option<usize>,
    timeout: ResponseTimeout,
}

impl<S> ResponseBody<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    /// Create `MessageBody` for request.
    pub fn new(res: &mut ClientResponse<S>) -> ResponseBody<S> {
        let length = match res.headers().get(&header::CONTENT_LENGTH) {
            Some(value) => {
                let len = value.to_str().ok().and_then(|s| s.parse::<usize>().ok());

                match len {
                    None => return Self::err(PayloadError::UnknownLength),
                    len => len,
                }
            }
            None => None,
        };

        ResponseBody {
            length,
            timeout: mem::take(&mut res.timeout),
            body: Ok(ReadBody::new(res.take_payload(), DEFAULT_BODY_LIMIT)),
        }
    }

    /// Change max size limit of payload.
    ///
    /// The default limit is 2 MiB.
    pub fn limit(mut self, limit: usize) -> Self {
        if let Ok(ref mut body) = self.body {
            body.limit = limit;
        }
        self
    }

    fn err(e: PayloadError) -> Self {
        ResponseBody {
            length: None,
            timeout: ResponseTimeout::default(),
            body: Err(Some(e)),
        }
    }
}

impl<S> Future for ResponseBody<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    type Output = Result<Bytes, PayloadError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match this.body {
            Err(ref mut err) => Poll::Ready(Err(err.take().unwrap())),
            Ok(ref mut body) => {
                if let Some(len) = this.length.take() {
                    if len > body.limit {
                        return Poll::Ready(Err(PayloadError::Overflow));
                    }
                }

                this.timeout.poll_timeout(cx)?;

                Pin::new(body).poll(cx)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use static_assertions::assert_impl_all;

    use super::*;
    use crate::{http::header, test::TestResponse};

    assert_impl_all!(ResponseBody<()>: Unpin);

    #[actix_rt::test]
    async fn test_body() {
        let mut req = TestResponse::with_header((header::CONTENT_LENGTH, "xxxx")).finish();
        match req.body().await.err().unwrap() {
            PayloadError::UnknownLength => {}
            _ => unreachable!("error"),
        }

        let mut req = TestResponse::with_header((header::CONTENT_LENGTH, "10000000")).finish();
        match req.body().await.err().unwrap() {
            PayloadError::Overflow => {}
            _ => unreachable!("error"),
        }

        let mut req = TestResponse::default()
            .set_payload(Bytes::from_static(b"test"))
            .finish();
        assert_eq!(req.body().await.ok().unwrap(), Bytes::from_static(b"test"));

        let mut req = TestResponse::default()
            .set_payload(Bytes::from_static(b"11111111111111"))
            .finish();
        match req.body().limit(5).await.err().unwrap() {
            PayloadError::Overflow => {}
            _ => unreachable!("error"),
        }
    }
}
