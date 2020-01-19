#![deny(rust_2018_idioms, warnings)]
#![allow(
    clippy::type_complexity,
    clippy::borrow_interior_mutable_const,
    clippy::needless_doctest_main
)]
//! An HTTP Client
//!
//! ```rust
//! use futures::future::{lazy, Future};
//! use actix_rt::System;
//! use awc::Client;
//!
//! #[actix_rt::main]
//! async fn main() {
//!    let mut client = Client::default();
//!
//!    let response = client.get("http://www.rust-lang.org") // <- Create request builder
//!        .header("User-Agent", "Actix-web")
//!        .send()                             // <- Send http request
//!        .await;
//!
//!     println!("Response: {:?}", response);
//! }
//! ```
use std::cell::RefCell;
use std::convert::TryFrom;
use std::rc::Rc;
use std::time::Duration;

pub use actix_http::{client::Connector, cookie, http};

use actix_http::http::{Error as HttpError, HeaderMap, Method, Uri};
use actix_http::RequestHead;

mod builder;
mod connect;
pub mod error;
mod frozen;
mod request;
mod response;
mod sender;
pub mod test;
pub mod ws;

pub use self::builder::ClientBuilder;
pub use self::connect::BoxedSocket;
pub use self::frozen::{FrozenClientRequest, FrozenSendBuilder};
pub use self::request::ClientRequest;
pub use self::response::{ClientResponse, JsonBody, MessageBody};
pub use self::sender::SendClientRequest;

use self::connect::{Connect, ConnectorWrapper};

/// An HTTP Client
///
/// ```rust
/// use awc::Client;
///
/// #[actix_rt::main]
/// async fn main() {
///     let mut client = Client::default();
///
///     let res = client.get("http://www.rust-lang.org") // <- Create request builder
///         .header("User-Agent", "Actix-web")
///         .send()                             // <- Send http request
///         .await;                             // <- send request and wait for response
///
///      println!("Response: {:?}", res);
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
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
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
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
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
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        self.request(Method::GET, url)
    }

    /// Construct HTTP *HEAD* request.
    pub fn head<U>(&self, url: U) -> ClientRequest
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        self.request(Method::HEAD, url)
    }

    /// Construct HTTP *PUT* request.
    pub fn put<U>(&self, url: U) -> ClientRequest
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        self.request(Method::PUT, url)
    }

    /// Construct HTTP *POST* request.
    pub fn post<U>(&self, url: U) -> ClientRequest
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        self.request(Method::POST, url)
    }

    /// Construct HTTP *PATCH* request.
    pub fn patch<U>(&self, url: U) -> ClientRequest
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        self.request(Method::PATCH, url)
    }

    /// Construct HTTP *DELETE* request.
    pub fn delete<U>(&self, url: U) -> ClientRequest
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        self.request(Method::DELETE, url)
    }

    /// Construct HTTP *OPTIONS* request.
    pub fn options<U>(&self, url: U) -> ClientRequest
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        self.request(Method::OPTIONS, url)
    }

    /// Construct WebSockets request.
    pub fn ws<U>(&self, url: U) -> ws::WebsocketsRequest
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        let mut req = ws::WebsocketsRequest::new(url, self.0.clone());
        for (key, value) in self.0.headers.iter() {
            req.head.headers.insert(key.clone(), value.clone());
        }
        req
    }
}
