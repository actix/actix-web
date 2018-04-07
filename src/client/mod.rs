//! HTTP client
mod connector;
mod parser;
mod request;
mod response;
mod pipeline;
mod writer;

pub use self::pipeline::{SendRequest, SendRequestError};
pub use self::request::{ClientRequest, ClientRequestBuilder};
pub use self::response::ClientResponse;
pub use self::connector::{
    Connect, Pause, Resume,
    Connection, ClientConnector, ClientConnectorError};
pub(crate) use self::writer::HttpClientWriter;
pub(crate) use self::parser::{HttpResponseParser, HttpResponseParserError};

use error::ResponseError;
use http::Method;
use httpresponse::HttpResponse;


/// Convert `SendRequestError` to a `HttpResponse`
impl ResponseError for SendRequestError {
    fn error_response(&self) -> HttpResponse {
        match *self {
            SendRequestError::Connector(_) => HttpResponse::BadGateway(),
            _ => HttpResponse::InternalServerError(),
        }
        .into()
    }
}

/// Create request builder for `GET` requests
///
/// ```rust
/// # extern crate actix;
/// # extern crate actix_web;
/// # extern crate futures;
/// # use futures::Future;
/// use actix_web::client;
///
/// fn main() {
///     let sys = actix::System::new("test");
///
///     actix::Arbiter::handle().spawn({
///         client::get("http://www.rust-lang.org")   // <- Create request builder
///             .header("User-Agent", "Actix-web")
///             .finish().unwrap()
///             .send()                               // <- Send http request
///             .map_err(|_| ())
///             .and_then(|response| {  // <- server http response
///                 println!("Response: {:?}", response);
/// #               actix::Arbiter::system().do_send(actix::msgs::SystemExit(0));
///                 Ok(())
///             })
///     });
///
///     sys.run();
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
