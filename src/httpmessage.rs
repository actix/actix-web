use bytes::{Bytes, BytesMut};
use encoding::all::UTF_8;
use encoding::label::encoding_from_whatwg_label;
use encoding::types::{DecoderTrap, Encoding};
use encoding::EncodingRef;
use futures::{Async, Future, Poll, Stream};
use http::{header, HeaderMap};
use mime::Mime;
use serde::de::DeserializeOwned;
use serde_urlencoded;
use std::str;

use error::{
    ContentTypeError, ParseError, PayloadError, ReadlinesError, UrlencodedError,
};
use header::Header;
use json::JsonBody;
use multipart::Multipart;

/// Trait that implements general purpose operations on http messages
pub trait HttpMessage: Sized {
    /// Type of message payload stream
    type Stream: Stream<Item = Bytes, Error = PayloadError> + Sized;

    /// Read the message headers.
    fn headers(&self) -> &HeaderMap;

    /// Message payload stream
    fn payload(&self) -> Self::Stream;

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
    fn body(&self) -> MessageBody<Self> {
        MessageBody::new(self)
    }

    /// Parse `application/x-www-form-urlencoded` encoded request's body.
    /// Return `UrlEncoded` future. Form can be deserialized to any type that
    /// implements `Deserialize` trait from *serde*.
    ///
    /// Returns error:
    ///
    /// * content type is not `application/x-www-form-urlencoded`
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
    fn urlencoded<T: DeserializeOwned>(&self) -> UrlEncoded<Self, T> {
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
    fn json<T: DeserializeOwned>(&self) -> JsonBody<Self, T> {
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
    /// # use actix_web::actix::fut::FinishStream;
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
    fn multipart(&self) -> Multipart<Self::Stream> {
        let boundary = Multipart::boundary(self.headers());
        Multipart::new(boundary, self.payload())
    }

    /// Return stream of lines.
    fn readlines(&self) -> Readlines<Self> {
        Readlines::new(self)
    }
}

/// Stream to read request line by line.
pub struct Readlines<T: HttpMessage> {
    stream: T::Stream,
    buff: BytesMut,
    limit: usize,
    checked_buff: bool,
    encoding: EncodingRef,
    err: Option<ReadlinesError>,
}

impl<T: HttpMessage> Readlines<T> {
    /// Create a new stream to read request line by line.
    fn new(req: &T) -> Self {
        let encoding = match req.encoding() {
            Ok(enc) => enc,
            Err(err) => return Self::err(req, err.into()),
        };

        Readlines {
            stream: req.payload(),
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

    fn err(req: &T, err: ReadlinesError) -> Self {
        Readlines {
            stream: req.payload(),
            buff: BytesMut::new(),
            limit: 262_144,
            checked_buff: true,
            encoding: UTF_8,
            err: Some(err),
        }
    }
}

impl<T: HttpMessage + 'static> Stream for Readlines<T> {
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
                let enc: *const Encoding = self.encoding as *const Encoding;
                let line = if enc == UTF_8 {
                    str::from_utf8(&self.buff.split_to(ind + 1))
                        .map_err(|_| ReadlinesError::EncodingError)?
                        .to_owned()
                } else {
                    self.encoding
                        .decode(&self.buff.split_to(ind + 1), DecoderTrap::Strict)
                        .map_err(|_| ReadlinesError::EncodingError)?
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
                    let enc: *const Encoding = self.encoding as *const Encoding;
                    let line = if enc == UTF_8 {
                        str::from_utf8(&bytes.split_to(ind + 1))
                            .map_err(|_| ReadlinesError::EncodingError)?
                            .to_owned()
                    } else {
                        self.encoding
                            .decode(&bytes.split_to(ind + 1), DecoderTrap::Strict)
                            .map_err(|_| ReadlinesError::EncodingError)?
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
                let enc: *const Encoding = self.encoding as *const Encoding;
                let line = if enc == UTF_8 {
                    str::from_utf8(&self.buff)
                        .map_err(|_| ReadlinesError::EncodingError)?
                        .to_owned()
                } else {
                    self.encoding
                        .decode(&self.buff, DecoderTrap::Strict)
                        .map_err(|_| ReadlinesError::EncodingError)?
                };
                self.buff.clear();
                Ok(Async::Ready(Some(line)))
            }
            Err(e) => Err(ReadlinesError::from(e)),
        }
    }
}

/// Future that resolves to a complete http message body.
pub struct MessageBody<T: HttpMessage> {
    limit: usize,
    length: Option<usize>,
    stream: Option<T::Stream>,
    err: Option<PayloadError>,
    fut: Option<Box<Future<Item = Bytes, Error = PayloadError>>>,
}

impl<T: HttpMessage> MessageBody<T> {
    /// Create `MessageBody` for request.
    pub fn new(req: &T) -> MessageBody<T> {
        let mut len = None;
        if let Some(l) = req.headers().get(header::CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    len = Some(l)
                } else {
                    return Self::err(PayloadError::UnknownLength);
                }
            } else {
                return Self::err(PayloadError::UnknownLength);
            }
        }

        MessageBody {
            limit: 262_144,
            length: len,
            stream: Some(req.payload()),
            fut: None,
            err: None,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    fn err(e: PayloadError) -> Self {
        MessageBody {
            stream: None,
            limit: 262_144,
            fut: None,
            err: Some(e),
            length: None,
        }
    }
}

impl<T> Future for MessageBody<T>
where
    T: HttpMessage + 'static,
{
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut {
            return fut.poll();
        }

        if let Some(err) = self.err.take() {
            return Err(err);
        }

        if let Some(len) = self.length.take() {
            if len > self.limit {
                return Err(PayloadError::Overflow);
            }
        }

        // future
        let limit = self.limit;
        self.fut = Some(Box::new(
            self.stream
                .take()
                .expect("Can not be used second time")
                .from_err()
                .fold(BytesMut::with_capacity(8192), move |mut body, chunk| {
                    if (body.len() + chunk.len()) > limit {
                        Err(PayloadError::Overflow)
                    } else {
                        body.extend_from_slice(&chunk);
                        Ok(body)
                    }
                })
                .map(|body| body.freeze()),
        ));
        self.poll()
    }
}

/// Future that resolves to a parsed urlencoded values.
pub struct UrlEncoded<T: HttpMessage, U> {
    stream: Option<T::Stream>,
    limit: usize,
    length: Option<usize>,
    encoding: EncodingRef,
    err: Option<UrlencodedError>,
    fut: Option<Box<Future<Item = U, Error = UrlencodedError>>>,
}

impl<T: HttpMessage, U> UrlEncoded<T, U> {
    /// Create a new future to URL encode a request
    pub fn new(req: &T) -> UrlEncoded<T, U> {
        // check content type
        if req.content_type().to_lowercase() != "application/x-www-form-urlencoded" {
            return Self::err(UrlencodedError::ContentType);
        }
        let encoding = match req.encoding() {
            Ok(enc) => enc,
            Err(_) => return Self::err(UrlencodedError::ContentType),
        };

        let mut len = None;
        if let Some(l) = req.headers().get(header::CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    len = Some(l)
                } else {
                    return Self::err(UrlencodedError::UnknownLength);
                }
            } else {
                return Self::err(UrlencodedError::UnknownLength);
            }
        };

        UrlEncoded {
            encoding,
            stream: Some(req.payload()),
            limit: 262_144,
            length: len,
            fut: None,
            err: None,
        }
    }

    fn err(e: UrlencodedError) -> Self {
        UrlEncoded {
            stream: None,
            limit: 262_144,
            fut: None,
            err: Some(e),
            length: None,
            encoding: UTF_8,
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
    T: HttpMessage + 'static,
    U: DeserializeOwned + 'static,
{
    type Item = U;
    type Error = UrlencodedError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut {
            return fut.poll();
        }

        if let Some(err) = self.err.take() {
            return Err(err);
        }

        // payload size
        let limit = self.limit;
        if let Some(len) = self.length.take() {
            if len > limit {
                return Err(UrlencodedError::Overflow);
            }
        }

        // future
        let encoding = self.encoding;
        let fut = self
            .stream
            .take()
            .expect("UrlEncoded could not be used second time")
            .from_err()
            .fold(BytesMut::with_capacity(8192), move |mut body, chunk| {
                if (body.len() + chunk.len()) > limit {
                    Err(UrlencodedError::Overflow)
                } else {
                    body.extend_from_slice(&chunk);
                    Ok(body)
                }
            })
            .and_then(move |body| {
                if (encoding as *const Encoding) == UTF_8 {
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
        self.poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding::all::ISO_8859_2;
    use encoding::Encoding;
    use futures::Async;
    use mime;
    use test::TestRequest;

    #[test]
    fn test_content_type() {
        let req = TestRequest::with_header("content-type", "text/plain").finish();
        assert_eq!(req.content_type(), "text/plain");
        let req =
            TestRequest::with_header("content-type", "application/json; charset=utf=8")
                .finish();
        assert_eq!(req.content_type(), "application/json");
        let req = TestRequest::default().finish();
        assert_eq!(req.content_type(), "");
    }

    #[test]
    fn test_mime_type() {
        let req = TestRequest::with_header("content-type", "application/json").finish();
        assert_eq!(req.mime_type().unwrap(), Some(mime::APPLICATION_JSON));
        let req = TestRequest::default().finish();
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
        let req = TestRequest::default().finish();
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
    fn test_chunked() {
        let req = TestRequest::default().finish();
        assert!(!req.chunked().unwrap());

        let req =
            TestRequest::with_header(header::TRANSFER_ENCODING, "chunked").finish();
        assert!(req.chunked().unwrap());

        let req = TestRequest::default()
            .header(
                header::TRANSFER_ENCODING,
                Bytes::from_static(b"some va\xadscc\xacas0xsdasdlue"),
            )
            .finish();
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
        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ).header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .finish();

        let result = req.urlencoded::<Info>().poll().ok().unwrap();
        assert_eq!(
            result,
            Async::Ready(Info {
                hello: "world".to_owned()
            })
        );

        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded; charset=utf-8",
        ).header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .finish();

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

        let req = TestRequest::default()
            .set_payload(Bytes::from_static(b"test"))
            .finish();
        match req.body().poll().ok().unwrap() {
            Async::Ready(bytes) => assert_eq!(bytes, Bytes::from_static(b"test")),
            _ => unreachable!("error"),
        }

        let req = TestRequest::default()
            .set_payload(Bytes::from_static(b"11111111111111"))
            .finish();
        match req.body().limit(5).poll().err().unwrap() {
            PayloadError::Overflow => (),
            _ => unreachable!("error"),
        }
    }

    #[test]
    fn test_readlines() {
        let req = TestRequest::default()
            .set_payload(Bytes::from_static(
                b"Lorem Ipsum is simply dummy text of the printing and typesetting\n\
                  industry. Lorem Ipsum has been the industry's standard dummy\n\
                  Contrary to popular belief, Lorem Ipsum is not simply random text.",
            ))
            .finish();
        let mut r = Readlines::new(&req);
        match r.poll().ok().unwrap() {
            Async::Ready(Some(s)) => assert_eq!(
                s,
                "Lorem Ipsum is simply dummy text of the printing and typesetting\n"
            ),
            _ => unreachable!("error"),
        }
        match r.poll().ok().unwrap() {
            Async::Ready(Some(s)) => assert_eq!(
                s,
                "industry. Lorem Ipsum has been the industry's standard dummy\n"
            ),
            _ => unreachable!("error"),
        }
        match r.poll().ok().unwrap() {
            Async::Ready(Some(s)) => assert_eq!(
                s,
                "Contrary to popular belief, Lorem Ipsum is not simply random text."
            ),
            _ => unreachable!("error"),
        }
    }
}
