//! An HTTP Client
//!
//! ```rust
//! # use futures::future::{Future, lazy};
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

pub use actix_http::{client::Connector, http};

use actix_http::http::{HeaderMap, HttpTryFrom, Method, Uri};
use actix_http::RequestHead;

mod builder;
mod connect;
pub mod error;
mod request;
mod response;
pub mod test;
mod ws;

pub use self::builder::ClientBuilder;
pub use self::request::ClientRequest;
pub use self::response::ClientResponse;
pub use self::ws::WebsocketsRequest;

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
pub struct Client {
    pub(crate) connector: Rc<RefCell<dyn Connect>>,
    pub(crate) config: Rc<ClientConfig>,
}

pub(crate) struct ClientConfig {
    pub(crate) headers: HeaderMap,
}

impl Default for Client {
    fn default() -> Self {
        Client {
            connector: Rc::new(RefCell::new(ConnectorWrapper(
                Connector::new().service(),
            ))),
            config: Rc::new(ClientConfig {
                headers: HeaderMap::new(),
            }),
        }
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
        let mut req = ClientRequest::new(method, url, self.connector.clone());

        for (key, value) in &self.config.headers {
            req.head.headers.insert(key.clone(), value.clone());
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
        for (key, value) in &head.headers {
            req.head.headers.insert(key.clone(), value.clone());
        }
        req
    }

    pub fn get<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::GET, url)
    }

    pub fn head<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::HEAD, url)
    }

    pub fn put<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::PUT, url)
    }

    pub fn post<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::POST, url)
    }

    pub fn patch<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::PATCH, url)
    }

    pub fn delete<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::DELETE, url)
    }

    pub fn options<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        self.request(Method::OPTIONS, url)
    }

    pub fn ws<U>(&self, url: U) -> WebsocketsRequest
    where
        Uri: HttpTryFrom<U>,
    {
        let mut req = WebsocketsRequest::new(url, self.connector.clone());

        for (key, value) in &self.config.headers {
            req.head.headers.insert(key.clone(), value.clone());
        }
        req
    }
}
