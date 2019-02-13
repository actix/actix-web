use bytes::BytesMut;
use futures::{Future, Poll, Stream};
use http::header::CONTENT_LENGTH;

use bytes::Bytes;
use mime;
use serde::de::DeserializeOwned;
use serde_json;

use crate::error::{JsonPayloadError, PayloadError};
use crate::httpmessage::HttpMessage;
use crate::payload::Payload;

/// Request payload json parser that resolves to a deserialized `T` value.
///
/// Returns error:
///
/// * content type is not `application/json`
/// * content length is greater than 256k
///
/// # Server example
///
/// ```rust,ignore
/// # extern crate actix_web;
/// # extern crate futures;
/// # #[macro_use] extern crate serde_derive;
/// use actix_web::{AsyncResponder, Error, HttpMessage, HttpRequest, Response};
/// use futures::future::Future;
///
/// #[derive(Deserialize, Debug)]
/// struct MyObj {
///     name: String,
/// }
///
/// fn index(mut req: HttpRequest) -> Box<Future<Item = Response, Error = Error>> {
///     req.json()                   // <- get JsonBody future
///        .from_err()
///        .and_then(|val: MyObj| {  // <- deserialized value
///            println!("==== BODY ==== {:?}", val);
///            Ok(Response::Ok().into())
///        }).responder()
/// }
/// # fn main() {}
/// ```
pub struct JsonBody<T: HttpMessage, U> {
    limit: usize,
    length: Option<usize>,
    stream: Payload<T::Stream>,
    err: Option<JsonPayloadError>,
    fut: Option<Box<Future<Item = U, Error = JsonPayloadError>>>,
}

impl<T, U> JsonBody<T, U>
where
    T: HttpMessage,
    T::Stream: Stream<Item = Bytes, Error = PayloadError> + 'static,
    U: DeserializeOwned + 'static,
{
    /// Create `JsonBody` for request.
    pub fn new(req: &mut T) -> Self {
        // check content-type
        let json = if let Ok(Some(mime)) = req.mime_type() {
            mime.subtype() == mime::JSON || mime.suffix() == Some(mime::JSON)
        } else {
            false
        };
        if !json {
            return JsonBody {
                limit: 262_144,
                length: None,
                stream: Payload::None,
                fut: None,
                err: Some(JsonPayloadError::ContentType),
            };
        }

        let mut len = None;
        if let Some(l) = req.headers().get(CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    len = Some(l)
                }
            }
        }

        JsonBody {
            limit: 262_144,
            length: len,
            stream: req.take_payload(),
            fut: None,
            err: None,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

impl<T, U> Future for JsonBody<T, U>
where
    T: HttpMessage,
    T::Stream: Stream<Item = Bytes, Error = PayloadError> + 'static,
    U: DeserializeOwned + 'static,
{
    type Item = U;
    type Error = JsonPayloadError;

    fn poll(&mut self) -> Poll<U, JsonPayloadError> {
        if let Some(ref mut fut) = self.fut {
            return fut.poll();
        }

        if let Some(err) = self.err.take() {
            return Err(err);
        }

        let limit = self.limit;
        if let Some(len) = self.length.take() {
            if len > limit {
                return Err(JsonPayloadError::Overflow);
            }
        }

        let fut = std::mem::replace(&mut self.stream, Payload::None)
            .from_err()
            .fold(BytesMut::with_capacity(8192), move |mut body, chunk| {
                if (body.len() + chunk.len()) > limit {
                    Err(JsonPayloadError::Overflow)
                } else {
                    body.extend_from_slice(&chunk);
                    Ok(body)
                }
            })
            .and_then(|body| Ok(serde_json::from_slice::<U>(&body)?));
        self.fut = Some(Box::new(fut));
        self.poll()
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures::Async;
    use http::header;
    use serde_derive::{Deserialize, Serialize};

    use super::*;
    use crate::test::TestRequest;

    impl PartialEq for JsonPayloadError {
        fn eq(&self, other: &JsonPayloadError) -> bool {
            match *self {
                JsonPayloadError::Overflow => match *other {
                    JsonPayloadError::Overflow => true,
                    _ => false,
                },
                JsonPayloadError::ContentType => match *other {
                    JsonPayloadError::ContentType => true,
                    _ => false,
                },
                _ => false,
            }
        }
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct MyObject {
        name: String,
    }

    #[test]
    fn test_json_body() {
        let mut req = TestRequest::default().finish();
        let mut json = req.json::<MyObject>();
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::ContentType);

        let mut req = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/text"),
            )
            .finish();
        let mut json = req.json::<MyObject>();
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::ContentType);

        let mut req = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            )
            .header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("10000"),
            )
            .finish();
        let mut json = req.json::<MyObject>().limit(100);
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::Overflow);

        let mut req = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            )
            .header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            )
            .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
            .finish();

        let mut json = req.json::<MyObject>();
        assert_eq!(
            json.poll().ok().unwrap(),
            Async::Ready(MyObject {
                name: "test".to_owned()
            })
        );
    }
}
