use bytes::{Bytes, BytesMut};
use encoding::all::UTF_8;
use encoding::label::encoding_from_whatwg_label;
use encoding::types::{DecoderTrap, Encoding};
use encoding::EncodingRef;
use futures::{Future, Poll, Stream};
use http::{header, HeaderMap};
use http_range::HttpRange;
use mime::Mime;
use serde::de::DeserializeOwned;
use serde_urlencoded;
use std::str;

use error::{
    ContentTypeError, HttpRangeError, ParseError, PayloadError, UrlencodedError,
};
use header::Header;
use json::JsonBody;
use multipart::Multipart;

/// Trait that implements general purpose operations on http messages
pub trait HttpMessage {
    /// Read the message headers.
    fn headers(&self) -> &HeaderMap;

    #[doc(hidden)]
    /// Get a header
    fn get_header<H: Header>(&self) -> Option<H>
    where
        Self: Sized,
    {
        if self.headers().contains_key(H::name()) {
            H::parse(self).ok()
        } else {
            None
        }
    }

    /// Read the request content type. If request does not contain
    /// *Content-Type* header, empty str get returned.
    fn content_type(&self) -> &str {
        if let Some(content_type) = self.headers().get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                return content_type.split(';').next().unwrap().trim();
            }
        }
        ""
    }

    /// Get content type encoding
    ///
    /// UTF-8 is used by default, If request charset is not set.
    fn encoding(&self) -> Result<EncodingRef, ContentTypeError> {
        if let Some(mime_type) = self.mime_type()? {
            if let Some(charset) = mime_type.get_param("charset") {
                if let Some(enc) = encoding_from_whatwg_label(charset.as_str()) {
                    Ok(enc)
                } else {
                    Err(ContentTypeError::UnknownEncoding)
                }
            } else {
                Ok(UTF_8)
            }
        } else {
            Ok(UTF_8)
        }
    }

    /// Convert the request content type to a known mime type.
    fn mime_type(&self) -> Result<Option<Mime>, ContentTypeError> {
        if let Some(content_type) = self.headers().get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                return match content_type.parse() {
                    Ok(mt) => Ok(Some(mt)),
                    Err(_) => Err(ContentTypeError::ParseError),
                };
            } else {
                return Err(ContentTypeError::ParseError);
            }
        }
        Ok(None)
    }

    /// Check if request has chunked transfer encoding
    fn chunked(&self) -> Result<bool, ParseError> {
        if let Some(encodings) = self.headers().get(header::TRANSFER_ENCODING) {
            if let Ok(s) = encodings.to_str() {
                Ok(s.to_lowercase().contains("chunked"))
            } else {
                Err(ParseError::Header)
            }
        } else {
            Ok(false)
        }
    }

    /// Parses Range HTTP header string as per RFC 2616.
    /// `size` is full size of response (file).
    fn range(&self, size: u64) -> Result<Vec<HttpRange>, HttpRangeError> {
        if let Some(range) = self.headers().get(header::RANGE) {
            HttpRange::parse(unsafe { str::from_utf8_unchecked(range.as_bytes()) }, size)
                .map_err(|e| e.into())
        } else {
            Ok(Vec::new())
        }
    }

    /// Load http message body.
    ///
    /// By default only 256Kb payload reads to a memory, then
    /// `PayloadError::Overflow` get returned. Use `MessageBody::limit()`
    /// method to change upper limit.
    ///
    /// ## Server example
    ///
    /// ```rust
    /// # extern crate bytes;
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// # #[macro_use] extern crate serde_derive;
    /// use actix_web::{
    ///     AsyncResponder, FutureResponse, HttpMessage, HttpRequest, HttpResponse,
    /// };
    /// use bytes::Bytes;
    /// use futures::future::Future;
    ///
    /// fn index(mut req: HttpRequest) -> FutureResponse<HttpResponse> {
    ///     req.body()                     // <- get Body future
    ///        .limit(1024)                // <- change max size of the body to a 1kb
    ///        .from_err()
    ///        .and_then(|bytes: Bytes| {  // <- complete body
    ///            println!("==== BODY ==== {:?}", bytes);
    ///            Ok(HttpResponse::Ok().into())
    ///        }).responder()
    /// }
    /// # fn main() {}
    /// ```
    fn body(self) -> MessageBody<Self>
    where
        Self: Stream<Item = Bytes, Error = PayloadError> + Sized,
    {
        MessageBody::new(self)
    }

    /// Parse `application/x-www-form-urlencoded` encoded request's body.
    /// Return `UrlEncoded` future. Form can be deserialized to any type that
    /// implements `Deserialize` trait from *serde*.
    ///
    /// Returns error:
    ///
    /// * content type is not `application/x-www-form-urlencoded`
    /// * transfer encoding is `chunked`.
    /// * content-length is greater than 256k
    ///
    /// ## Server example
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// # use futures::Future;
    /// # use std::collections::HashMap;
    /// use actix_web::{FutureResponse, HttpMessage, HttpRequest, HttpResponse};
    ///
    /// fn index(mut req: HttpRequest) -> FutureResponse<HttpResponse> {
    ///     Box::new(
    ///         req.urlencoded::<HashMap<String, String>>()  // <- get UrlEncoded future
    ///            .from_err()
    ///            .and_then(|params| {  // <- url encoded parameters
    ///                println!("==== BODY ==== {:?}", params);
    ///                Ok(HttpResponse::Ok().into())
    ///           }),
    ///     )
    /// }
    /// # fn main() {}
    /// ```
    fn urlencoded<T: DeserializeOwned>(self) -> UrlEncoded<Self, T>
    where
        Self: Stream<Item = Bytes, Error = PayloadError> + Sized,
    {
        UrlEncoded::new(self)
    }

    /// Parse `application/json` encoded body.
    /// Return `JsonBody<T>` future. It resolves to a `T` value.
    ///
    /// Returns error:
    ///
    /// * content type is not `application/json`
    /// * content length is greater than 256k
    ///
    /// ## Server example
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// # #[macro_use] extern crate serde_derive;
    /// use actix_web::*;
    /// use futures::future::{ok, Future};
    ///
    /// #[derive(Deserialize, Debug)]
    /// struct MyObj {
    ///     name: String,
    /// }
    ///
    /// fn index(mut req: HttpRequest) -> Box<Future<Item = HttpResponse, Error = Error>> {
    ///     req.json()                   // <- get JsonBody future
    ///        .from_err()
    ///        .and_then(|val: MyObj| {  // <- deserialized value
    ///            println!("==== BODY ==== {:?}", val);
    ///            Ok(HttpResponse::Ok().into())
    ///        }).responder()
    /// }
    /// # fn main() {}
    /// ```
    fn json<T: DeserializeOwned>(self) -> JsonBody<Self, T>
    where
        Self: Stream<Item = Bytes, Error = PayloadError> + Sized,
    {
        JsonBody::new(self)
    }

    /// Return stream to http payload processes as multipart.
    ///
    /// Content-type: multipart/form-data;
    ///
    /// ## Server example
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # extern crate env_logger;
    /// # extern crate futures;
    /// # use std::str;
    /// # use actix_web::*;
    /// # use actix_web::actix::*;
    /// # use futures::{Future, Stream};
    /// # use futures::future::{ok, result, Either};
    /// fn index(mut req: HttpRequest) -> Box<Future<Item = HttpResponse, Error = Error>> {
    ///     req.multipart().from_err()       // <- get multipart stream for current request
    ///        .and_then(|item| match item { // <- iterate over multipart items
    ///            multipart::MultipartItem::Field(field) => {
    ///                // Field in turn is stream of *Bytes* object
    ///                Either::A(field.from_err()
    ///                          .map(|c| println!("-- CHUNK: \n{:?}", str::from_utf8(&c)))
    ///                          .finish())
    ///             },
    ///             multipart::MultipartItem::Nested(mp) => {
    ///                 // Or item could be nested Multipart stream
    ///                 Either::B(ok(()))
    ///             }
    ///         })
    ///         .finish()  // <- Stream::finish() combinator from actix
    ///         .map(|_| HttpResponse::Ok().into())
    ///         .responder()
    /// }
    /// # fn main() {}
    /// ```
    fn multipart(self) -> Multipart<Self>
    where
        Self: Stream<Item = Bytes, Error = PayloadError> + Sized,
    {
        let boundary = Multipart::boundary(self.headers());
        Multipart::new(boundary, self)
    }
}

/// Future that resolves to a complete http message body.
pub struct MessageBody<T> {
    limit: usize,
    req: Option<T>,
    fut: Option<Box<Future<Item = Bytes, Error = PayloadError>>>,
}

impl<T> MessageBody<T> {
    /// Create `RequestBody` for request.
    pub fn new(req: T) -> MessageBody<T> {
        MessageBody {
            limit: 262_144,
            req: Some(req),
            fut: None,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

impl<T> Future for MessageBody<T>
where
    T: HttpMessage + Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(req) = self.req.take() {
            if let Some(len) = req.headers().get(header::CONTENT_LENGTH) {
                if let Ok(s) = len.to_str() {
                    if let Ok(len) = s.parse::<usize>() {
                        if len > self.limit {
                            return Err(PayloadError::Overflow);
                        }
                    } else {
                        return Err(PayloadError::UnknownLength);
                    }
                } else {
                    return Err(PayloadError::UnknownLength);
                }
            }

            // future
            let limit = self.limit;
            self.fut = Some(Box::new(
                req.from_err()
                    .fold(BytesMut::new(), move |mut body, chunk| {
                        if (body.len() + chunk.len()) > limit {
                            Err(PayloadError::Overflow)
                        } else {
                            body.extend_from_slice(&chunk);
                            Ok(body)
                        }
                    })
                    .map(|body| body.freeze()),
            ));
        }

        self.fut
            .as_mut()
            .expect("UrlEncoded could not be used second time")
            .poll()
    }
}

/// Future that resolves to a parsed urlencoded values.
pub struct UrlEncoded<T, U> {
    req: Option<T>,
    limit: usize,
    fut: Option<Box<Future<Item = U, Error = UrlencodedError>>>,
}

impl<T, U> UrlEncoded<T, U> {
    pub fn new(req: T) -> UrlEncoded<T, U> {
        UrlEncoded {
            req: Some(req),
            limit: 262_144,
            fut: None,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

impl<T, U> Future for UrlEncoded<T, U>
where
    T: HttpMessage + Stream<Item = Bytes, Error = PayloadError> + 'static,
    U: DeserializeOwned + 'static,
{
    type Item = U;
    type Error = UrlencodedError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(req) = self.req.take() {
            if req.chunked().unwrap_or(false) {
                return Err(UrlencodedError::Chunked);
            } else if let Some(len) = req.headers().get(header::CONTENT_LENGTH) {
                if let Ok(s) = len.to_str() {
                    if let Ok(len) = s.parse::<u64>() {
                        if len > 262_144 {
                            return Err(UrlencodedError::Overflow);
                        }
                    } else {
                        return Err(UrlencodedError::UnknownLength);
                    }
                } else {
                    return Err(UrlencodedError::UnknownLength);
                }
            }

            // check content type
            if req.content_type().to_lowercase() != "application/x-www-form-urlencoded" {
                return Err(UrlencodedError::ContentType);
            }
            let encoding = req.encoding().map_err(|_| UrlencodedError::ContentType)?;

            // future
            let limit = self.limit;
            let fut = req
                .from_err()
                .fold(BytesMut::new(), move |mut body, chunk| {
                    if (body.len() + chunk.len()) > limit {
                        Err(UrlencodedError::Overflow)
                    } else {
                        body.extend_from_slice(&chunk);
                        Ok(body)
                    }
                })
                .and_then(move |body| {
                    let enc: *const Encoding = encoding as *const Encoding;
                    if enc == UTF_8 {
                        serde_urlencoded::from_bytes::<U>(&body)
                            .map_err(|_| UrlencodedError::Parse)
                    } else {
                        let body = encoding
                            .decode(&body, DecoderTrap::Strict)
                            .map_err(|_| UrlencodedError::Parse)?;
                        serde_urlencoded::from_str::<U>(&body)
                            .map_err(|_| UrlencodedError::Parse)
                    }
                });
            self.fut = Some(Box::new(fut));
        }

        self.fut
            .as_mut()
            .expect("UrlEncoded could not be used second time")
            .poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding::all::ISO_8859_2;
    use encoding::Encoding;
    use futures::Async;
    use http::{Method, Uri, Version};
    use httprequest::HttpRequest;
    use mime;
    use std::str::FromStr;
    use test::TestRequest;

    #[test]
    fn test_content_type() {
        let req = TestRequest::with_header("content-type", "text/plain").finish();
        assert_eq!(req.content_type(), "text/plain");
        let req =
            TestRequest::with_header("content-type", "application/json; charset=utf=8")
                .finish();
        assert_eq!(req.content_type(), "application/json");
        let req = HttpRequest::default();
        assert_eq!(req.content_type(), "");
    }

    #[test]
    fn test_mime_type() {
        let req = TestRequest::with_header("content-type", "application/json").finish();
        assert_eq!(req.mime_type().unwrap(), Some(mime::APPLICATION_JSON));
        let req = HttpRequest::default();
        assert_eq!(req.mime_type().unwrap(), None);
        let req =
            TestRequest::with_header("content-type", "application/json; charset=utf-8")
                .finish();
        let mt = req.mime_type().unwrap().unwrap();
        assert_eq!(mt.get_param(mime::CHARSET), Some(mime::UTF_8));
        assert_eq!(mt.type_(), mime::APPLICATION);
        assert_eq!(mt.subtype(), mime::JSON);
    }

    #[test]
    fn test_mime_type_error() {
        let req = TestRequest::with_header(
            "content-type",
            "applicationadfadsfasdflknadsfklnadsfjson",
        ).finish();
        assert_eq!(Err(ContentTypeError::ParseError), req.mime_type());
    }

    #[test]
    fn test_encoding() {
        let req = HttpRequest::default();
        assert_eq!(UTF_8.name(), req.encoding().unwrap().name());

        let req = TestRequest::with_header("content-type", "application/json").finish();
        assert_eq!(UTF_8.name(), req.encoding().unwrap().name());

        let req = TestRequest::with_header(
            "content-type",
            "application/json; charset=ISO-8859-2",
        ).finish();
        assert_eq!(ISO_8859_2.name(), req.encoding().unwrap().name());
    }

    #[test]
    fn test_encoding_error() {
        let req = TestRequest::with_header("content-type", "applicatjson").finish();
        assert_eq!(Some(ContentTypeError::ParseError), req.encoding().err());

        let req = TestRequest::with_header(
            "content-type",
            "application/json; charset=kkkttktk",
        ).finish();
        assert_eq!(
            Some(ContentTypeError::UnknownEncoding),
            req.encoding().err()
        );
    }

    #[test]
    fn test_no_request_range_header() {
        let req = HttpRequest::default();
        let ranges = req.range(100).unwrap();
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_request_range_header() {
        let req = TestRequest::with_header(header::RANGE, "bytes=0-4").finish();
        let ranges = req.range(100).unwrap();
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].length, 5);
    }

    #[test]
    fn test_chunked() {
        let req = HttpRequest::default();
        assert!(!req.chunked().unwrap());

        let req =
            TestRequest::with_header(header::TRANSFER_ENCODING, "chunked").finish();
        assert!(req.chunked().unwrap());

        let mut headers = HeaderMap::new();
        let s = unsafe {
            str::from_utf8_unchecked(b"some va\xadscc\xacas0xsdasdlue".as_ref())
        };

        headers.insert(
            header::TRANSFER_ENCODING,
            header::HeaderValue::from_str(s).unwrap(),
        );
        let req = HttpRequest::new(
            Method::GET,
            Uri::from_str("/").unwrap(),
            Version::HTTP_11,
            headers,
            None,
        );
        assert!(req.chunked().is_err());
    }

    impl PartialEq for UrlencodedError {
        fn eq(&self, other: &UrlencodedError) -> bool {
            match *self {
                UrlencodedError::Chunked => match *other {
                    UrlencodedError::Chunked => true,
                    _ => false,
                },
                UrlencodedError::Overflow => match *other {
                    UrlencodedError::Overflow => true,
                    _ => false,
                },
                UrlencodedError::UnknownLength => match *other {
                    UrlencodedError::UnknownLength => true,
                    _ => false,
                },
                UrlencodedError::ContentType => match *other {
                    UrlencodedError::ContentType => true,
                    _ => false,
                },
                _ => false,
            }
        }
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct Info {
        hello: String,
    }

    #[test]
    fn test_urlencoded_error() {
        let req =
            TestRequest::with_header(header::TRANSFER_ENCODING, "chunked").finish();
        assert_eq!(
            req.urlencoded::<Info>().poll().err().unwrap(),
            UrlencodedError::Chunked
        );

        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ).header(header::CONTENT_LENGTH, "xxxx")
            .finish();
        assert_eq!(
            req.urlencoded::<Info>().poll().err().unwrap(),
            UrlencodedError::UnknownLength
        );

        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ).header(header::CONTENT_LENGTH, "1000000")
            .finish();
        assert_eq!(
            req.urlencoded::<Info>().poll().err().unwrap(),
            UrlencodedError::Overflow
        );

        let req = TestRequest::with_header(header::CONTENT_TYPE, "text/plain")
            .header(header::CONTENT_LENGTH, "10")
            .finish();
        assert_eq!(
            req.urlencoded::<Info>().poll().err().unwrap(),
            UrlencodedError::ContentType
        );
    }

    #[test]
    fn test_urlencoded() {
        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ).header(header::CONTENT_LENGTH, "11")
            .finish();
        req.payload_mut()
            .unread_data(Bytes::from_static(b"hello=world"));

        let result = req.urlencoded::<Info>().poll().ok().unwrap();
        assert_eq!(
            result,
            Async::Ready(Info {
                hello: "world".to_owned()
            })
        );

        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded; charset=utf-8",
        ).header(header::CONTENT_LENGTH, "11")
            .finish();
        req.payload_mut()
            .unread_data(Bytes::from_static(b"hello=world"));

        let result = req.urlencoded().poll().ok().unwrap();
        assert_eq!(
            result,
            Async::Ready(Info {
                hello: "world".to_owned()
            })
        );
    }

    #[test]
    fn test_message_body() {
        let req = TestRequest::with_header(header::CONTENT_LENGTH, "xxxx").finish();
        match req.body().poll().err().unwrap() {
            PayloadError::UnknownLength => (),
            _ => unreachable!("error"),
        }

        let req = TestRequest::with_header(header::CONTENT_LENGTH, "1000000").finish();
        match req.body().poll().err().unwrap() {
            PayloadError::Overflow => (),
            _ => unreachable!("error"),
        }

        let mut req = HttpRequest::default();
        req.payload_mut().unread_data(Bytes::from_static(b"test"));
        match req.body().poll().ok().unwrap() {
            Async::Ready(bytes) => assert_eq!(bytes, Bytes::from_static(b"test")),
            _ => unreachable!("error"),
        }

        let mut req = HttpRequest::default();
        req.payload_mut()
            .unread_data(Bytes::from_static(b"11111111111111"));
        match req.body().limit(5).poll().err().unwrap() {
            PayloadError::Overflow => (),
            _ => unreachable!("error"),
        }
    }
}
