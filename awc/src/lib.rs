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

use actix_http::http::{HttpTryFrom, Method, Uri};
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
}

impl Default for Client {
    fn default() -> Self {
        Client {
            connector: Rc::new(RefCell::new(ConnectorWrapper(
                Connector::new().service(),
            ))),
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
        ClientRequest::new(method, url, self.connector.clone())
    }

    /// Create `ClientRequest` from `RequestHead`
    ///
    /// It is useful for proxy requests. This implementation
    /// copies all headers and the method.
    pub fn request_from<U>(&self, url: U, head: &RequestHead) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        let mut req =
            ClientRequest::new(head.method.clone(), url, self.connector.clone());

        for (key, value) in &head.headers {
            req.head.headers.insert(key.clone(), value.clone());
        }

        req
    }

    pub fn get<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        ClientRequest::new(Method::GET, url, self.connector.clone())
    }

    pub fn head<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        ClientRequest::new(Method::HEAD, url, self.connector.clone())
    }

    pub fn put<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        ClientRequest::new(Method::PUT, url, self.connector.clone())
    }

    pub fn post<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        ClientRequest::new(Method::POST, url, self.connector.clone())
    }

    pub fn patch<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        ClientRequest::new(Method::PATCH, url, self.connector.clone())
    }

    pub fn delete<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        ClientRequest::new(Method::DELETE, url, self.connector.clone())
    }

    pub fn options<U>(&self, url: U) -> ClientRequest
    where
        Uri: HttpTryFrom<U>,
    {
        ClientRequest::new(Method::OPTIONS, url, self.connector.clone())
    }

    pub fn ws<U>(&self, url: U) -> WebsocketsRequest
    where
        Uri: HttpTryFrom<U>,
    {
        WebsocketsRequest::new(url, self.connector.clone())
    }
}
