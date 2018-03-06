//! Http client
mod connector;
mod parser;
mod request;
mod response;
mod pipeline;
mod writer;

pub use self::pipeline::{SendRequest, SendRequestError};
pub use self::request::{ClientRequest, ClientRequestBuilder};
pub use self::response::ClientResponse;
pub use self::connector::{Connect, Connection, ClientConnector, ClientConnectorError};
pub(crate) use self::writer::HttpClientWriter;
pub(crate) use self::parser::{HttpResponseParser, HttpResponseParserError};


use httpcodes;
use httpresponse::HttpResponse;
use error::ResponseError;


/// Convert `SendRequestError` to a `HttpResponse`
impl ResponseError for SendRequestError {

    fn error_response(&self) -> HttpResponse {
        match *self {
            SendRequestError::Connector(_) => httpcodes::HttpBadGateway.into(),
            _ => httpcodes::HttpInternalServerError.into(),
        }
    }
}
