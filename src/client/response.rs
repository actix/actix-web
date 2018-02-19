use std::{fmt, str};
use std::rc::Rc;
use std::cell::UnsafeCell;
use std::collections::HashMap;

use bytes::{Bytes, BytesMut};
use cookie::Cookie;
use futures::{Async, Future, Poll, Stream};
use http::{HeaderMap, StatusCode, Version};
use http::header::{self, HeaderValue};
use mime::Mime;
use serde_json;
use serde::de::DeserializeOwned;
use url::form_urlencoded;

// use multipart::Multipart;
use error::{CookieParseError, ParseError, PayloadError, JsonPayloadError, UrlencodedError};

use super::pipeline::Pipeline;


pub(crate) struct ClientMessage {
    pub status: StatusCode,
    pub version: Version,
    pub headers: HeaderMap<HeaderValue>,
    pub cookies: Option<Vec<Cookie<'static>>>,
}

impl Default for ClientMessage {

    fn default() -> ClientMessage {
        ClientMessage {
            status: StatusCode::OK,
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(16),
            cookies: None,
        }
    }
}

/// An HTTP Client response
pub struct ClientResponse(Rc<UnsafeCell<ClientMessage>>, Option<Box<Pipeline>>);

impl ClientResponse {

    pub(crate) fn new(msg: ClientMessage) -> ClientResponse {
        ClientResponse(Rc::new(UnsafeCell::new(msg)), None)
    }

    pub(crate) fn set_pipeline(&mut self, pl: Box<Pipeline>) {
        self.1 = Some(pl);
    }

    #[inline]
    fn as_ref(&self) -> &ClientMessage {
        unsafe{ &*self.0.get() }
    }

    #[inline]
    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref))]
    fn as_mut(&self) -> &mut ClientMessage {
        unsafe{ &mut *self.0.get() }
    }

    /// Get the HTTP version of this response.
    #[inline]
    pub fn version(&self) -> Version {
        self.as_ref().version
    }

    /// Get the headers from the response.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.as_ref().headers
    }

    /// Get a mutable reference to the headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.as_mut().headers
    }

    /// Get the status from the server.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.as_ref().status
    }

    /// Set the `StatusCode` for this response.
    #[inline]
    pub fn set_status(&mut self, status: StatusCode) {
        self.as_mut().status = status
    }

    /// Load request cookies.
    pub fn cookies(&self) -> Result<&Vec<Cookie<'static>>, CookieParseError> {
        if self.as_ref().cookies.is_none() {
            let msg = self.as_mut();
            let mut cookies = Vec::new();
            if let Some(val) = msg.headers.get(header::COOKIE) {
                let s = str::from_utf8(val.as_bytes())
                    .map_err(CookieParseError::from)?;
                for cookie in s.split("; ") {
                    cookies.push(Cookie::parse_encoded(cookie)?.into_owned());
                }
            }
            msg.cookies = Some(cookies)
        }
        Ok(self.as_ref().cookies.as_ref().unwrap())
    }

    /// Return request cookie.
    pub fn cookie(&self, name: &str) -> Option<&Cookie> {
        if let Ok(cookies) = self.cookies() {
            for cookie in cookies {
                if cookie.name() == name {
                    return Some(cookie)
                }
            }
        }
        None
    }

    /// Read the request content type. If request does not contain
    /// *Content-Type* header, empty str get returned.
    pub fn content_type(&self) -> &str {
        if let Some(content_type) = self.headers().get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                return content_type.split(';').next().unwrap().trim()
            }
        }
        ""
    }

    /// Convert the request content type to a known mime type.
    pub fn mime_type(&self) -> Option<Mime> {
        if let Some(content_type) = self.headers().get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                return match content_type.parse() {
                    Ok(mt) => Some(mt),
                    Err(_) => None
                };
            }
        }
        None
    }

    /// Check if request has chunked transfer encoding
    pub fn chunked(&self) -> Result<bool, ParseError> {
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

    /// Load request body.
    ///
    /// By default only 256Kb payload reads to a memory, then connection get dropped
    /// and `PayloadError` get returned. Use `ResponseBody::limit()`
    /// method to change upper limit.
    pub fn body(self) -> ResponseBody {
        ResponseBody::new(self)
    }

    // /// Return stream to http payload processes as multipart.
    // ///
    // /// Content-type: multipart/form-data;
    // pub fn multipart(mut self) -> Multipart {
    //      Multipart::from_response(&mut self)
    // }

    /// Parse `application/x-www-form-urlencoded` encoded body.
    /// Return `UrlEncoded` future. It resolves to a `HashMap<String, String>` which
    /// contains decoded parameters.
    ///
    /// Returns error:
    ///
    /// * content type is not `application/x-www-form-urlencoded`
    /// * transfer encoding is `chunked`.
    /// * content-length is greater than 256k
    pub fn urlencoded(self) -> UrlEncoded {
        UrlEncoded::new(self)
    }

    /// Parse `application/json` encoded body.
    /// Return `JsonResponse<T>` future. It resolves to a `T` value.
    ///
    /// Returns error:
    ///
    /// * content type is not `application/json`
    /// * content length is greater than 256k
    pub fn json<T: DeserializeOwned>(self) -> JsonResponse<T> {
        JsonResponse::from_response(self)
    }
}

impl fmt::Debug for ClientResponse {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(
            f, "\nClientResponse {:?} {}\n", self.version(), self.status());
        let _ = write!(f, "  headers:\n");
        for key in self.headers().keys() {
            let vals: Vec<_> = self.headers().get_all(key).iter().collect();
            if vals.len() > 1 {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals);
            } else {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals[0]);
            }
        }
        res
    }
}

/// Future that resolves to a complete request body.
impl Stream for ClientResponse {
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some(ref mut pl) = self.1 {
            pl.poll()
        } else {
            Ok(Async::Ready(None))
        }
    }
}

/// Future that resolves to a complete response body.
#[must_use = "ResponseBody does nothing unless polled"]
pub struct ResponseBody {
    limit: usize,
    resp: Option<ClientResponse>,
    fut: Option<Box<Future<Item=Bytes, Error=PayloadError>>>,
}

impl ResponseBody {

    /// Create `ResponseBody` for request.
    pub fn new(resp: ClientResponse) -> Self {
        ResponseBody {
            limit: 262_144,
            resp: Some(resp),
            fut: None,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

impl Future for ResponseBody {
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Bytes, PayloadError> {
        if let Some(resp) = self.resp.take() {
            if let Some(len) = resp.headers().get(header::CONTENT_LENGTH) {
                if let Ok(s) = len.to_str() {
                    if let Ok(len) = s.parse::<usize>() {
                        if len > self.limit {
                            return Err(PayloadError::Overflow);
                        }
                    } else {
                        return Err(PayloadError::Overflow);
                    }
                }
            }
            let limit = self.limit;
            let fut = resp.from_err()
                .fold(BytesMut::new(), move |mut body, chunk| {
                    if (body.len() + chunk.len()) > limit {
                        Err(PayloadError::Overflow)
                    } else {
                        body.extend_from_slice(&chunk);
                        Ok(body)
                    }
                })
                .map(|bytes| bytes.freeze());
            self.fut = Some(Box::new(fut));
        }

        self.fut.as_mut().expect("ResponseBody could not be used second time").poll()
    }
}

/// Client response json parser that resolves to a deserialized `T` value.
///
/// Returns error:
///
/// * content type is not `application/json`
/// * content length is greater than 256k
#[must_use = "JsonResponse does nothing unless polled"]
pub struct JsonResponse<T: DeserializeOwned>{
    limit: usize,
    ct: &'static str,
    resp: Option<ClientResponse>,
    fut: Option<Box<Future<Item=T, Error=JsonPayloadError>>>,
}

impl<T: DeserializeOwned> JsonResponse<T> {

    /// Create `JsonResponse` for request.
    pub fn from_response(resp: ClientResponse) -> Self {
        JsonResponse{
            limit: 262_144,
            resp: Some(resp),
            ct: "application/json",
            fut: None,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set allowed content type.
    ///
    /// By default *application/json* content type is used. Set content type
    /// to empty string if you want to disable content type check.
    pub fn content_type(mut self, ct: &'static str) -> Self {
        self.ct = ct;
        self
    }
}

impl<T: DeserializeOwned + 'static> Future for JsonResponse<T> {
    type Item = T;
    type Error = JsonPayloadError;

    fn poll(&mut self) -> Poll<T, JsonPayloadError> {
        if let Some(resp) = self.resp.take() {
            if let Some(len) = resp.headers().get(header::CONTENT_LENGTH) {
                if let Ok(s) = len.to_str() {
                    if let Ok(len) = s.parse::<usize>() {
                        if len > self.limit {
                            return Err(JsonPayloadError::Overflow);
                        }
                    } else {
                        return Err(JsonPayloadError::Overflow);
                    }
                }
            }
            // check content-type
            if !self.ct.is_empty() && resp.content_type() != self.ct {
                return Err(JsonPayloadError::ContentType)
            }

            let limit = self.limit;
            let fut = resp.from_err()
                .fold(BytesMut::new(), move |mut body, chunk| {
                    if (body.len() + chunk.len()) > limit {
                        Err(JsonPayloadError::Overflow)
                    } else {
                        body.extend_from_slice(&chunk);
                        Ok(body)
                    }
                })
                .and_then(|body| Ok(serde_json::from_slice::<T>(&body)?));

            self.fut = Some(Box::new(fut));
        }

        self.fut.as_mut().expect("JsonResponse could not be used second time").poll()
    }
}

/// Future that resolves to a parsed urlencoded values.
#[must_use = "UrlEncoded does nothing unless polled"]
pub struct UrlEncoded {
    resp: Option<ClientResponse>,
    limit: usize,
    fut: Option<Box<Future<Item=HashMap<String, String>, Error=UrlencodedError>>>,
}

impl UrlEncoded {
    pub fn new(resp: ClientResponse) -> UrlEncoded {
        UrlEncoded{resp: Some(resp),
                   limit: 262_144,
                   fut: None}
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

impl Future for UrlEncoded {
    type Item = HashMap<String, String>;
    type Error = UrlencodedError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(resp) = self.resp.take() {
            if resp.chunked().unwrap_or(false) {
                return Err(UrlencodedError::Chunked)
            } else if let Some(len) = resp.headers().get(header::CONTENT_LENGTH) {
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
            let mut encoding = false;
            if let Some(content_type) = resp.headers().get(header::CONTENT_TYPE) {
                if let Ok(content_type) = content_type.to_str() {
                    if content_type.to_lowercase() == "application/x-www-form-urlencoded" {
                        encoding = true;
                    }
                }
            }
            if !encoding {
                return Err(UrlencodedError::ContentType);
            }

            // urlencoded body
            let limit = self.limit;
            let fut = resp.from_err()
                .fold(BytesMut::new(), move |mut body, chunk| {
                    if (body.len() + chunk.len()) > limit {
                        Err(UrlencodedError::Overflow)
                    } else {
                        body.extend_from_slice(&chunk);
                        Ok(body)
                    }
                })
                .and_then(|body| {
                    let mut m = HashMap::new();
                    for (k, v) in form_urlencoded::parse(&body) {
                        m.insert(k.into(), v.into());
                    }
                    Ok(m)
                });

            self.fut = Some(Box::new(fut));
        }

        self.fut.as_mut().expect("UrlEncoded could not be used second time").poll()
    }
}
