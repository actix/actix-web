//! HTTP client.

use std::{rc::Rc, time::Duration};

use actix_http::{error::HttpError, header::HeaderMap, Method, RequestHead, Uri};
use actix_rt::net::TcpStream;
use actix_service::Service;
pub use actix_tls::connect::{
    ConnectError as TcpConnectError, ConnectInfo, Connection as TcpConnection,
};

use crate::{ws, BoxConnectorService, ClientBuilder, ClientRequest};

mod config;
mod connection;
mod connector;
mod error;
mod h1proto;
mod h2proto;
mod pool;

pub use self::{
    connection::{Connection, ConnectionIo},
    connector::{Connector, ConnectorService},
    error::{ConnectError, FreezeRequestError, InvalidUrl, SendRequestError},
};

#[derive(Clone)]
pub struct Connect {
    pub uri: Uri,
    pub addr: Option<std::net::SocketAddr>,
}

/// An asynchronous HTTP and WebSocket client.
///
/// You should take care to create, at most, one `Client` per thread. Otherwise, expect higher CPU
/// and memory usage.
///
/// # Examples
/// ```
/// use awc::Client;
///
/// #[actix_rt::main]
/// async fn main() {
///     let mut client = Client::default();
///
///     let res = client.get("http://www.rust-lang.org")
///         .insert_header(("User-Agent", "my-app/1.2"))
///         .send()
///         .await;
///
///      println!("Response: {:?}", res);
/// }
/// ```
#[derive(Clone)]
pub struct Client(pub(crate) ClientConfig);

#[derive(Clone)]
pub(crate) struct ClientConfig {
    pub(crate) connector: BoxConnectorService,
    pub(crate) default_headers: Rc<HeaderMap>,
    pub(crate) timeout: Option<Duration>,
}

impl Default for Client {
    fn default() -> Self {
        ClientBuilder::new().finish()
    }
}

impl Client {
    /// Constructs new client instance with default settings.
    pub fn new() -> Client {
        Client::default()
    }

    /// Constructs new `Client` builder.
    ///
    /// This function is equivalent of `ClientBuilder::new()`.
    pub fn builder() -> ClientBuilder<
        impl Service<
                ConnectInfo<Uri>,
                Response = TcpConnection<Uri, TcpStream>,
                Error = TcpConnectError,
            > + Clone,
    > {
        ClientBuilder::new()
    }

    /// Construct HTTP request.
    pub fn request<U>(&self, method: Method, url: U) -> ClientRequest
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        let mut req = ClientRequest::new(method, url, self.0.clone());

        for header in self.0.default_headers.iter() {
            req = req.append_header(header);
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
        for header in head.headers.iter() {
            req = req.insert_header_if_none(header);
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

    /// Initialize a WebSocket connection.
    /// Returns a WebSocket connection builder.
    pub fn ws<U>(&self, url: U) -> ws::WebsocketsRequest
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        let mut req = ws::WebsocketsRequest::new(url, self.0.clone());
        for (key, value) in self.0.default_headers.iter() {
            req.head.headers.insert(key.clone(), value.clone());
        }
        req
    }

    /// Get default HeaderMap of Client.
    ///
    /// Returns Some(&mut HeaderMap) when Client object is unique
    /// (No other clone of client exists at the same time).
    pub fn headers(&mut self) -> Option<&mut HeaderMap> {
        Rc::get_mut(&mut self.0.default_headers)
    }
}
