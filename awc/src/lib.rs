//! `awc` is a HTTP and WebSocket client library built on the Actix ecosystem.
//!
//! ## Making a GET request
//!
//! ```no_run
//! # #[actix_rt::main]
//! # async fn main() -> Result<(), awc::error::SendRequestError> {
//! let mut client = awc::Client::default();
//! let response = client.get("http://www.rust-lang.org") // <- Create request builder
//!     .insert_header(("User-Agent", "Actix-web"))
//!     .send()                                            // <- Send http request
//!     .await?;
//!
//!  println!("Response: {:?}", response);
//! # Ok(())
//! # }
//! ```
//!
//! ## Making POST requests
//!
//! ### Raw body contents
//!
//! ```no_run
//! # #[actix_rt::main]
//! # async fn main() -> Result<(), awc::error::SendRequestError> {
//! let mut client = awc::Client::default();
//! let response = client.post("http://httpbin.org/post")
//!     .send_body("Raw body contents")
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Forms
//!
//! ```no_run
//! # #[actix_rt::main]
//! # async fn main() -> Result<(), awc::error::SendRequestError> {
//! let params = [("foo", "bar"), ("baz", "quux")];
//!
//! let mut client = awc::Client::default();
//! let response = client.post("http://httpbin.org/post")
//!     .send_form(&params)
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### JSON
//!
//! ```no_run
//! # #[actix_rt::main]
//! # async fn main() -> Result<(), awc::error::SendRequestError> {
//! let request = serde_json::json!({
//!     "lang": "rust",
//!     "body": "json"
//! });
//!
//! let mut client = awc::Client::default();
//! let response = client.post("http://httpbin.org/post")
//!     .send_json(&request)
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## WebSocket support
//!
//! ```no_run
//! # #[actix_rt::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use futures_util::{sink::SinkExt, stream::StreamExt};
//! let (_resp, mut connection) = awc::Client::new()
//!     .ws("ws://echo.websocket.org")
//!     .connect()
//!     .await?;
//!
//! connection
//!     .send(awc::ws::Message::Text("Echo".into()))
//!     .await?;
//! let response = connection.next().await.unwrap()?;
//! # assert_eq!(response, awc::ws::Frame::Text("Echo".as_bytes().into()));
//! # Ok(())
//! # }
//! ```

#![deny(rust_2018_idioms)]
#![allow(
    clippy::type_complexity,
    clippy::borrow_interior_mutable_const,
    clippy::needless_doctest_main
)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

use std::{convert::TryFrom, rc::Rc, time::Duration};

#[cfg(feature = "cookies")]
pub use cookie;

pub use actix_http::{client::Connector, http};

use actix_http::{
    client::{TcpConnect, TcpConnectError, TcpConnection},
    http::{Error as HttpError, HeaderMap, Method, Uri},
    RequestHead,
};
use actix_rt::net::TcpStream;
use actix_service::Service;

mod builder;
mod connect;
pub mod error;
mod frozen;
pub mod middleware;
mod request;
mod response;
mod sender;
pub mod test;
pub mod ws;

pub use self::builder::ClientBuilder;
pub use self::connect::{BoxConnectorService, BoxedSocket, ConnectRequest, ConnectResponse};
pub use self::frozen::{FrozenClientRequest, FrozenSendBuilder};
pub use self::request::ClientRequest;
pub use self::response::{ClientResponse, JsonBody, MessageBody};
pub use self::sender::SendClientRequest;

/// An asynchronous HTTP and WebSocket client.
///
/// ## Examples
///
/// ```
/// use awc::Client;
///
/// #[actix_rt::main]
/// async fn main() {
///     let mut client = Client::default();
///
///     let res = client.get("http://www.rust-lang.org") // <- Create request builder
///         .insert_header(("User-Agent", "Actix-web"))
///         .send()                             // <- Send HTTP request
///         .await;                             // <- send request and wait for response
///
///      println!("Response: {:?}", res);
/// }
/// ```
#[derive(Clone)]
pub struct Client(ClientConfig);

#[derive(Clone)]
pub(crate) struct ClientConfig {
    pub(crate) connector: BoxConnectorService,
    pub(crate) headers: Rc<HeaderMap>,
    pub(crate) timeout: Option<Duration>,
}

impl Default for Client {
    fn default() -> Self {
        ClientBuilder::new().finish()
    }
}

impl Client {
    /// Create new client instance with default settings.
    pub fn new() -> Client {
        Client::default()
    }

    /// Create `Client` builder.
    /// This function is equivalent of `ClientBuilder::new()`.
    pub fn builder() -> ClientBuilder<
        impl Service<
                TcpConnect<Uri>,
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

        for header in self.0.headers.iter() {
            req = req.insert_header_if_none(header);
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
        for (key, value) in self.0.headers.iter() {
            req.head.headers.insert(key.clone(), value.clone());
        }
        req
    }

    /// Get default HeaderMap of Client.
    ///
    /// Returns Some(&mut HeaderMap) when Client object is unique
    /// (No other clone of client exists at the same time).
    pub fn headers(&mut self) -> Option<&mut HeaderMap> {
        Rc::get_mut(&mut self.0.headers)
    }
}
