use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use actix_http::client::Connector;
use actix_http::http::{header::IntoHeaderValue, HeaderMap, HeaderName, HttpTryFrom};

use crate::connect::{Connect, ConnectorWrapper};
use crate::Client;

/// An HTTP Client builder
///
/// This type can be used to construct an instance of `Client` through a
/// builder-like pattern.
pub struct ClientBuilder {
    connector: Rc<RefCell<dyn Connect>>,
    default_headers: bool,
    allow_redirects: bool,
    max_redirects: usize,
    headers: HeaderMap,
}

impl ClientBuilder {
    pub fn new() -> Self {
        ClientBuilder {
            default_headers: true,
            allow_redirects: true,
            max_redirects: 10,
            headers: HeaderMap::new(),
            connector: Rc::new(RefCell::new(ConnectorWrapper(
                Connector::new().service(),
            ))),
        }
    }

    /// Do not follow redirects.
    ///
    /// Redirects are allowed by default.
    pub fn disable_redirects(mut self) -> Self {
        self.allow_redirects = false;
        self
    }

    /// Set max number of redirects.
    ///
    /// Max redirects is set to 10 by default.
    pub fn max_redirects(mut self, num: usize) -> Self {
        self.max_redirects = num;
        self
    }

    /// Do not add default request headers.
    /// By default `Accept-Encoding` and `User-Agent` headers are set.
    pub fn skip_default_headers(mut self) -> Self {
        self.default_headers = false;
        self
    }

    /// Add default header. This header adds to every request.
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        <HeaderName as HttpTryFrom<K>>::Error: fmt::Debug,
        V: IntoHeaderValue,
        V::Error: fmt::Debug,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match value.try_into() {
                Ok(value) => {
                    self.headers.append(key, value);
                }
                Err(e) => log::error!("Header value error: {:?}", e),
            },
            Err(e) => log::error!("Header name error: {:?}", e),
        }
        self
    }

    /// Finish build process and create `Client`.
    pub fn finish(self) -> Client {
        Client {
            connector: self.connector,
        }
    }
}
