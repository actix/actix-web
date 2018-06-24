use std::{fmt, str};

use bytes::Bytes;
use cookie::Cookie;
use futures::{Async, Poll, Stream};
use http::header::{self, HeaderValue};
use http::{HeaderMap, StatusCode, Version};

use error::{CookieParseError, PayloadError};
use httpmessage::HttpMessage;

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
pub struct ClientResponse(ClientMessage, Option<Box<Pipeline>>);

impl HttpMessage for ClientResponse {
    /// Get the headers from the response.
    #[inline]
    fn headers(&self) -> &HeaderMap {
        &self.0.headers
    }
}

impl ClientResponse {
    pub(crate) fn new(msg: ClientMessage) -> ClientResponse {
        ClientResponse(msg, None)
    }

    pub(crate) fn set_pipeline(&mut self, pl: Box<Pipeline>) {
        self.1 = Some(pl);
    }

    /// Get the HTTP version of this response.
    #[inline]
    pub fn version(&self) -> Version {
        self.0.version
    }

    /// Get the status from the server.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.0.status
    }

    /// Load response cookies.
    pub fn cookies(&self) -> Result<Vec<Cookie<'static>>, CookieParseError> {
        let mut cookies = Vec::new();
        for val in self.0.headers.get_all(header::SET_COOKIE).iter() {
            let s = str::from_utf8(val.as_bytes()).map_err(CookieParseError::from)?;
            cookies.push(Cookie::parse_encoded(s)?.into_owned());
        }
        Ok(cookies)
    }

    /// Return request cookie.
    pub fn cookie(&self, name: &str) -> Option<Cookie> {
        if let Ok(cookies) = self.cookies() {
            for cookie in cookies {
                if cookie.name() == name {
                    return Some(cookie);
                }
            }
        }
        None
    }
}

impl fmt::Debug for ClientResponse {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = writeln!(f, "\nClientResponse {:?} {}", self.version(), self.status());
        let _ = writeln!(f, "  headers:");
        for (key, val) in self.headers().iter() {
            let _ = writeln!(f, "    {:?}: {:?}", key, val);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug() {
        let mut resp = ClientResponse::new(ClientMessage::default());
        resp.0
            .headers
            .insert(header::COOKIE, HeaderValue::from_static("cookie1=value1"));
        resp.0
            .headers
            .insert(header::COOKIE, HeaderValue::from_static("cookie2=value2"));

        let dbg = format!("{:?}", resp);
        assert!(dbg.contains("ClientResponse"));
    }
}
