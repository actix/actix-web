use std::{
    future::Future,
    marker::PhantomData,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::{error::PayloadError, header, HttpMessage};
use bytes::Bytes;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;
use serde::de::DeserializeOwned;

use super::{read_body::ReadBody, ResponseTimeout, DEFAULT_BODY_LIMIT};
use crate::{error::JsonPayloadError, ClientResponse};

pin_project! {
    /// A `Future` that reads a body stream, parses JSON, resolving to a deserialized `T`.
    ///
    /// # Errors
    /// `Future` implementation returns error if:
    /// - content type is not `application/json`;
    /// - content length is greater than [limit](JsonBody::limit) (default: 2 MiB).
    pub struct JsonBody<S, T> {
        #[pin]
        body: Option<ReadBody<S>>,
        length: Option<usize>,
        timeout: ResponseTimeout,
        err: Option<JsonPayloadError>,
        _phantom: PhantomData<T>,
    }
}

impl<S, T> JsonBody<S, T>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
    T: DeserializeOwned,
{
    /// Creates a JSON body stream reader from a response by taking its payload.
    pub fn new(res: &mut ClientResponse<S>) -> Self {
        // check content-type
        let json = if let Ok(Some(mime)) = res.mime_type() {
            mime.subtype() == mime::JSON || mime.suffix() == Some(mime::JSON)
        } else {
            false
        };

        if !json {
            return JsonBody {
                length: None,
                body: None,
                timeout: ResponseTimeout::default(),
                err: Some(JsonPayloadError::ContentType),
                _phantom: PhantomData,
            };
        }

        let length = res
            .headers()
            .get(&header::CONTENT_LENGTH)
            .and_then(|len_hdr| len_hdr.to_str().ok())
            .and_then(|len_str| len_str.parse::<usize>().ok());

        JsonBody {
            body: Some(ReadBody::new(res.take_payload(), DEFAULT_BODY_LIMIT)),
            length,
            timeout: mem::take(&mut res.timeout),
            err: None,
            _phantom: PhantomData,
        }
    }

    /// Change max size of payload. Default limit is 2 MiB.
    pub fn limit(mut self, limit: usize) -> Self {
        if let Some(ref mut fut) = self.body {
            fut.limit = limit;
        }

        self
    }
}

impl<S, T> Future for JsonBody<S, T>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
    T: DeserializeOwned,
{
    type Output = Result<T, JsonPayloadError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if let Some(err) = this.err.take() {
            return Poll::Ready(Err(err));
        }

        if let Some(len) = this.length.take() {
            let body = Option::as_ref(&this.body).unwrap();
            if len > body.limit {
                return Poll::Ready(Err(JsonPayloadError::Payload(PayloadError::Overflow)));
            }
        }

        this.timeout
            .poll_timeout(cx)
            .map_err(JsonPayloadError::Payload)?;

        let body = ready!(this.body.as_pin_mut().unwrap().poll(cx))?;
        Poll::Ready(serde_json::from_slice::<T>(&body).map_err(JsonPayloadError::from))
    }
}

#[cfg(test)]
mod tests {
    use actix_http::BoxedPayloadStream;
    use serde::{Deserialize, Serialize};
    use static_assertions::assert_impl_all;

    use super::*;
    use crate::test::TestResponse;

    assert_impl_all!(JsonBody<BoxedPayloadStream, String>: Unpin);

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct MyObject {
        name: String,
    }

    fn json_eq(err: JsonPayloadError, other: JsonPayloadError) -> bool {
        match err {
            JsonPayloadError::Payload(PayloadError::Overflow) => {
                matches!(other, JsonPayloadError::Payload(PayloadError::Overflow))
            }
            JsonPayloadError::ContentType => matches!(other, JsonPayloadError::ContentType),
            _ => false,
        }
    }

    #[actix_rt::test]
    async fn read_json_body() {
        let mut req = TestResponse::default().finish();
        let json = JsonBody::<_, MyObject>::new(&mut req).await;
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::ContentType));

        let mut req = TestResponse::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/text"),
            ))
            .finish();
        let json = JsonBody::<_, MyObject>::new(&mut req).await;
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::ContentType));

        let mut req = TestResponse::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("10000"),
            ))
            .finish();

        let json = JsonBody::<_, MyObject>::new(&mut req).limit(100).await;
        assert!(json_eq(
            json.err().unwrap(),
            JsonPayloadError::Payload(PayloadError::Overflow)
        ));

        let mut req = TestResponse::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ))
            .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
            .finish();

        let json = JsonBody::<_, MyObject>::new(&mut req).await;
        assert_eq!(
            json.ok().unwrap(),
            MyObject {
                name: "test".to_owned()
            }
        );
    }
}
