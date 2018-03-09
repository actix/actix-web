use bytes::{Bytes, BytesMut};
use futures::{Poll, Future, Stream};
use http::header::CONTENT_LENGTH;

use bytes::IntoBuf;
use prost::Message;

use error::{Error, ProtoBufPayloadError, PayloadError};
use handler::Responder;
use httpmessage::HttpMessage;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;


#[derive(Debug)]
pub struct ProtoBuf<T: Message>(pub T);

impl<T: Message> Responder for ProtoBuf<T> {
    type Item = HttpResponse;
    type Error = Error;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, Error> {
        let mut buf = Vec::new();
        self.0.encode(&mut buf)
              .map_err(Error::from)
              .and_then(|()| {
                Ok(HttpResponse::Ok()
                    .content_type("application/protobuf")
                    .body(buf)
                    .into())
              })
    }
}




pub struct ProtoBufBody<T, U: Message + Default>{
    limit: usize,
    ct: &'static str,
    req: Option<T>,
    fut: Option<Box<Future<Item=U, Error=ProtoBufPayloadError>>>,
}

impl<T, U: Message + Default> ProtoBufBody<T, U> {

    /// Create `ProtoBufBody` for request.
    pub fn new(req: T) -> Self {
        ProtoBufBody{
            limit: 262_144,
            req: Some(req),
            fut: None,
            ct: "application/protobuf",
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set allowed content type.
    ///
    /// By default *application/protobuf* content type is used. Set content type
    /// to empty string if you want to disable content type check.
    pub fn content_type(mut self, ct: &'static str) -> Self {
        self.ct = ct;
        self
    }
}

impl<T, U: Message + Default + 'static> Future for ProtoBufBody<T, U>
    where T: HttpMessage + Stream<Item=Bytes, Error=PayloadError> + 'static
{
    type Item = U;
    type Error = ProtoBufPayloadError;

    fn poll(&mut self) -> Poll<U, ProtoBufPayloadError> {
        if let Some(req) = self.req.take() {
            if let Some(len) = req.headers().get(CONTENT_LENGTH) {
                if let Ok(s) = len.to_str() {
                    if let Ok(len) = s.parse::<usize>() {
                        if len > self.limit {
                            return Err(ProtoBufPayloadError::Overflow);
                        }
                    } else {
                        return Err(ProtoBufPayloadError::Overflow);
                    }
                }
            }
            // check content-type
            if !self.ct.is_empty() && req.content_type() != self.ct {
                return Err(ProtoBufPayloadError::ContentType)
            }

            let limit = self.limit;
            let fut = req.from_err()
                .fold(BytesMut::new(), move |mut body, chunk| {
                    if (body.len() + chunk.len()) > limit {
                        Err(ProtoBufPayloadError::Overflow)
                    } else {
                        body.extend_from_slice(&chunk);
                        Ok(body)
                    }
                })
                .and_then(|body| Ok(<U>::decode(&mut body.into_buf())?));
            self.fut = Some(Box::new(fut));
        }

        self.fut.as_mut().expect("ProtoBufBody could not be used second time").poll()
    }
}