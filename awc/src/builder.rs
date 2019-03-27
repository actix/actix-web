use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use actix_http::client::{ConnectError, Connection, Connector};
use actix_http::http::{
    header::IntoHeaderValue, HeaderMap, HeaderName, HttpTryFrom, Uri,
};
use actix_service::Service;

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

    /// Use custom connector service.
    pub fn connector<T>(mut self, connector: T) -> Self
    where
        T: Service<Request = Uri, Error = ConnectError> + 'static,
        T::Response: Connection,
        <T::Response as Connection>::Future: 'static,
        T::Future: 'static,
    {
        self.connector = Rc::new(RefCell::new(ConnectorWrapper(connector)));
        self
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
    /// By default `Date` and `User-Agent` headers are set.
    pub fn no_default_headers(mut self) -> Self {
        self.default_headers = false;
        self
    }

    /// Add default header. Headers adds byt this method
    /// get added to every request.
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

    /// Finish build process and create `Client` instance.
    pub fn finish(self) -> Client {
        Client {
            connector: self.connector,
        }
    }
}
