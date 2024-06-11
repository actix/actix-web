use std::{
    future::Future,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::{error::PayloadError, header, HttpMessage};
use bytes::Bytes;
use futures_core::Stream;
use pin_project_lite::pin_project;

use super::{read_body::ReadBody, ResponseTimeout, DEFAULT_BODY_LIMIT};
use crate::ClientResponse;

pin_project! {
    /// A `Future` that reads a body stream, resolving as [`Bytes`].
    ///
    /// # Errors
    /// `Future` implementation returns error if:
    /// - content type is not `application/json`;
    /// - content length is greater than [limit](JsonBody::limit) (default: 2 MiB).
    pub struct ResponseBody<S> {
        #[pin]
        body: Option<ReadBody<S>>,
        length: Option<usize>,
        timeout: ResponseTimeout,
        err: Option<PayloadError>,
    }
}

#[deprecated(since = "3.0.0", note = "Renamed to `ResponseBody`.")]
pub type MessageBody<B> = ResponseBody<B>;

impl<S> ResponseBody<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    /// Creates a body stream reader from a response by taking its payload.
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
            body: Some(ReadBody::new(res.take_payload(), DEFAULT_BODY_LIMIT)),
            length,
            timeout: mem::take(&mut res.timeout),
            err: None,
        }
    }

    /// Change max size limit of payload.
    ///
    /// The default limit is 2 MiB.
    pub fn limit(mut self, limit: usize) -> Self {
        if let Some(ref mut body) = self.body {
            body.limit = limit;
        }

        self
    }

    fn err(err: PayloadError) -> Self {
        ResponseBody {
            body: None,
            length: None,
            timeout: ResponseTimeout::default(),
            err: Some(err),
        }
    }
}

impl<S> Future for ResponseBody<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    type Output = Result<Bytes, PayloadError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if let Some(err) = this.err.take() {
            return Poll::Ready(Err(err));
        }

        if let Some(len) = this.length.take() {
            let body = Option::as_ref(&this.body).unwrap();
            if len > body.limit {
                return Poll::Ready(Err(PayloadError::Overflow));
            }
        }

        this.timeout.poll_timeout(cx)?;

        this.body.as_pin_mut().unwrap().poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use static_assertions::assert_impl_all;

    use super::*;
    use crate::test::TestResponse;

    assert_impl_all!(ResponseBody<()>: Unpin);

    #[actix_rt::test]
    async fn read_body() {
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
