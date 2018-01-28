#![allow(dead_code)]
use std::fmt;
use http::{HeaderMap, StatusCode, Version};
use http::header::HeaderValue;

use payload::Payload;


pub struct ClientResponse {
    /// The response's status
    status: StatusCode,

    /// The response's version
    version: Version,

    /// The response's headers
    headers: HeaderMap<HeaderValue>,

    payload: Option<Payload>,
}

impl ClientResponse {
    pub fn new(status: StatusCode, version: Version,
               headers: HeaderMap<HeaderValue>, payload: Option<Payload>) -> Self {
        ClientResponse {
            status: status, version: version, headers: headers, payload: payload
        }
    }

 /// Get the HTTP version of this response.
    #[inline]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Get the headers from the response.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get a mutable reference to the headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Get the status from the server.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Set the `StatusCode` for this response.
    #[inline]
    pub fn status_mut(&mut self) -> &mut StatusCode {
        &mut self.status
    }
}

impl fmt::Debug for ClientResponse {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(
            f, "\nClientResponse {:?} {}\n", self.version, self.status);
        let _ = write!(f, "  headers:\n");
        for key in self.headers.keys() {
            let vals: Vec<_> = self.headers.get_all(key).iter().collect();
            if vals.len() > 1 {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals);
            } else {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals[0]);
            }
        }
        res
    }
}
