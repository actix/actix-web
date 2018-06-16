//! Http client api
//!
//! ```rust
//! # extern crate actix_web;
//! # extern crate futures;
//! # extern crate tokio;
//! # use futures::Future;
//! # use std::process;
//! use actix_web::client;
//!
//! fn main() {
//!     let mut sys = actix_web::actix::System::new("test");
//!
//!     sys.block_on(
//!         client::get("http://www.rust-lang.org")   // <- Create request builder
//!             .header("User-Agent", "Actix-web")
//!             .finish().unwrap()
//!             .send()                               // <- Send http request
//!             .map_err(|_| ())
//!             .and_then(|response| {                // <- server http response
//!                 println!("Response: {:?}", response);
//!                 Ok(())
//!             })
//!     );
//! }
//! ```
mod body;
mod connector;
mod parser;
mod pipeline;
mod request;
mod response;
mod writer;

pub use self::body::{ClientBody, ClientBodyStream};
pub use self::connector::{
    ClientConnector, ClientConnectorError, ClientConnectorStats, Connect, Connection,
    Pause, Resume,
};
pub(crate) use self::parser::{HttpResponseParser, HttpResponseParserError};
pub use self::pipeline::{SendRequest, SendRequestError};
pub use self::request::{ClientRequest, ClientRequestBuilder};
pub use self::response::ClientResponse;
pub(crate) use self::writer::HttpClientWriter;

use error::ResponseError;
use http::Method;
use httpresponse::HttpResponse;

/// Convert `SendRequestError` to a `HttpResponse`
impl ResponseError for SendRequestError {
    fn error_response(&self) -> HttpResponse {
        match *self {
            SendRequestError::Timeout => HttpResponse::GatewayTimeout(),
            SendRequestError::Connector(_) => HttpResponse::BadGateway(),
            _ => HttpResponse::InternalServerError(),
        }.into()
    }
}

/// Create request builder for `GET` requests
///
///
/// ```rust
/// # extern crate actix_web;
/// # extern crate futures;
/// # extern crate tokio;
/// # extern crate env_logger;
/// # use futures::Future;
/// # use std::process;
/// use actix_web::client;
///
/// fn main() {
///     tokio::run(
///         client::get("http://www.rust-lang.org")   // <- Create request builder
///             .header("User-Agent", "Actix-web")
///             .finish().unwrap()
///             .send()                               // <- Send http request
///             .map_err(|_| ())
///             .and_then(|response| {                // <- server http response
///                 println!("Response: {:?}", response);
/// #               process::exit(0);
///                 Ok(())
///             }),
///     );
/// }
/// ```
pub fn get<U: AsRef<str>>(uri: U) -> ClientRequestBuilder {
    let mut builder = ClientRequest::build();
    builder.method(Method::GET).uri(uri);
    builder
}

/// Create request builder for `HEAD` requests
pub fn head<U: AsRef<str>>(uri: U) -> ClientRequestBuilder {
    let mut builder = ClientRequest::build();
    builder.method(Method::HEAD).uri(uri);
    builder
}

/// Create request builder for `POST` requests
pub fn post<U: AsRef<str>>(uri: U) -> ClientRequestBuilder {
    let mut builder = ClientRequest::build();
    builder.method(Method::POST).uri(uri);
    builder
}

/// Create request builder for `PUT` requests
pub fn put<U: AsRef<str>>(uri: U) -> ClientRequestBuilder {
    let mut builder = ClientRequest::build();
    builder.method(Method::PUT).uri(uri);
    builder
}

/// Create request builder for `DELETE` requests
pub fn delete<U: AsRef<str>>(uri: U) -> ClientRequestBuilder {
    let mut builder = ClientRequest::build();
    builder.method(Method::DELETE).uri(uri);
    builder
}
