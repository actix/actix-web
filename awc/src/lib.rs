use std::cell::RefCell;
use std::rc::Rc;

pub use actix_http::client::{ConnectError, InvalidUrl, SendRequestError};
pub use actix_http::error::PayloadError;
pub use actix_http::http;

use actix_http::client::Connector;
use actix_http::http::{HttpTryFrom, Method, Uri};

mod builder;
mod connect;
mod request;
mod response;

pub use self::builder::ClientBuilder;
pub use self::request::ClientRequest;
pub use self::response::ClientResponse;

use self::connect::{Connect, ConnectorWrapper};

/// An HTTP Client Request
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
}
