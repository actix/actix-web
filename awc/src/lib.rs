#![allow(clippy::borrow_interior_mutable_const)]
//! An HTTP Client
//!
//! ```rust
//! use futures::future::{lazy, Future};
//! use actix_rt::System;
//! use awc::Client;
//!
//! fn main() {
//!     System::new("test").block_on(lazy(|| {
//!        let mut client = Client::default();
//!
//!        client.get("http://www.rust-lang.org") // <- Create request builder
//!           .header("User-Agent", "Actix-web")
//!           .send()                             // <- Send http request
//!           .map_err(|_| ())
//!           .and_then(|response| {              // <- server http response
//!                println!("Response: {:?}", response);
//!                Ok(())
//!           })
//!     }));
//! }
//! ```
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub use actix_http::{client::Connector, cookie, http};

use actix_http::http::{HeaderMap, HttpTryFrom, Method, Uri};
use actix_http::RequestHead;

mod builder;
mod connect;
pub mod error;
mod request;
mod response;
pub mod test;
pub mod ws;

pub use self::builder::ClientBuilder;
pub use self::connect::BoxedSocket;
pub use self::request::ClientRequest;
pub use self::response::{ClientResponse, JsonBody, MessageBody};

use self::connect::{Connect, ConnectorWrapper};

/// An HTTP Client
///
/// ```rust
/// # use futures::future::{Future, lazy};
/// use actix_rt::System;
/// use awc::Client;
///
/// fn main() {
///     System::new("test").block_on(lazy(|| {
///        let mut client = Client::default();
///
///        client.get("http://www.rust-lang.org") // <- Create request builder
///           .header("User-Agent", "Actix-web")
///           .send()                             // <- Send http request
///           .map_err(|_| ())
///           .and_then(|response| {              // <- server http response
///                println!("Response: {:?}", response);
///                Ok(())
///           })
///     }));
/// }
/// ```
#[derive(Clone)]
pub struct Client(Rc<ClientConfig>);

pub(crate) struct ClientConfig {
    pub(crate) connector: RefCell<Box<dyn Connect>>,
    pub(crate) headers: HeaderMap,
    pub(crate) timeout: Option<Duration>,
}

impl Default for Client {
    fn default() -> Self {
        Client(Rc::new(ClientConfig {
            connector: RefCell::new(Box::new(ConnectorWrapper(
                Connector::new().finish(),
            ))),
            headers: HeaderMap::new(),
            timeout: Some(Duration::from_secs(5)),
        }))
    }
}

impl Client {
    /// Create new client instance with default settings.
    pub fn new() -> Client {
        Client::default()
    }

    /// Build client instance.
    pub fn build() -> ClientBuilder {
        ClientBuilder::new()
    }

    /// Construct HTTP request.
    pub fn request<U>(&self, method: Method, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        let mut req = ClientRequest::new(method, url, self.0.clone());

        for (key, value) in self.0.headers.iter() {
            req = req.set_header_if_none(key.clone(), value.clone());
        }
        req
    }

    /// Create `ClientRequest` from `RequestHead`
    ///
    /// It is useful for proxy requests. This implementation
    /// copies all headers and the method.
    pub fn request_from<U>(&self, url: U, head: &RequestHead) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        let mut req = self.request(head.method.clone(), url);
        for (key, value) in head.headers.iter() {
            req = req.set_header_if_none(key.clone(), value.clone());
        }
        req
    }

    /// Construct HTTP *GET* request.
    pub fn get<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::GET, url)
    }

    /// Construct HTTP *HEAD* request.
    pub fn head<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::HEAD, url)
    }

    /// Construct HTTP *PUT* request.
    pub fn put<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::PUT, url)
    }

    /// Construct HTTP *POST* request.
    pub fn post<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::POST, url)
    }

    /// Construct HTTP *PATCH* request.
    pub fn patch<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::PATCH, url)
    }

    /// Construct HTTP *DELETE* request.
    pub fn delete<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::DELETE, url)
    }

    /// Construct HTTP *OPTIONS* request.
    pub fn options<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::OPTIONS, url)
    }

    /// Construct WebSockets request.
    pub fn ws<U>(&self, url: U) -> ws::WebsocketsRequest
    where
        Uri: HttpTryFrom<U>,
    {
        let mut req = ws::WebsocketsRequest::new(url, self.0.clone());
        for (key, value) in self.0.headers.iter() {
            req.head.headers.insert(key.clone(), value.clone());
        }
        req
    }
}
