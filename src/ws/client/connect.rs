//! Http client request
use std::str;

use cookie::Cookie;
use http::header::{HeaderName, HeaderValue};
use http::{Error as HttpError, HttpTryFrom};

use client::{ClientRequest, ClientRequestBuilder};
use header::IntoHeaderValue;

use super::ClientError;

/// `WebSocket` connection
pub struct Connect {
    pub(super) request: ClientRequestBuilder,
    pub(super) err: Option<ClientError>,
    pub(super) http_err: Option<HttpError>,
    pub(super) origin: Option<HeaderValue>,
    pub(super) protocols: Option<String>,
    pub(super) max_size: usize,
    pub(super) server_mode: bool,
}

impl Connect {
    /// Create new websocket connection
    pub fn new<S: AsRef<str>>(uri: S) -> Connect {
        let mut cl = Connect {
            request: ClientRequest::build(),
            err: None,
            http_err: None,
            origin: None,
            protocols: None,
            max_size: 65_536,
            server_mode: false,
        };
        cl.request.uri(uri.as_ref());
        cl
    }

    /// Set supported websocket protocols
    pub fn protocols<U, V>(mut self, protos: U) -> Self
    where
        U: IntoIterator<Item = V> + 'static,
        V: AsRef<str>,
    {
        let mut protos = protos
            .into_iter()
            .fold(String::new(), |acc, s| acc + s.as_ref() + ",");
        protos.pop();
        self.protocols = Some(protos);
        self
    }

    /// Set cookie for handshake request
    pub fn cookie(mut self, cookie: Cookie) -> Self {
        self.request.cookie(cookie);
        self
    }

    /// Set request Origin
    pub fn origin<V>(mut self, origin: V) -> Self
    where
        HeaderValue: HttpTryFrom<V>,
    {
        match HeaderValue::try_from(origin) {
            Ok(value) => self.origin = Some(value),
            Err(e) => self.http_err = Some(e.into()),
        }
        self
    }

    /// Set max frame size
    ///
    /// By default max size is set to 64kb
    pub fn max_frame_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    /// Disable payload masking. By default ws client masks frame payload.
    pub fn server_mode(mut self) -> Self {
        self.server_mode = true;
        self
    }

    /// Set request header
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        self.request.header(key, value);
        self
    }
}
