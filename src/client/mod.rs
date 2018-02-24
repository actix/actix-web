//! Http client
mod connector;
mod encoding;
mod parser;
mod request;
mod response;
mod pipeline;
mod writer;

pub use self::pipeline::{SendRequest, SendRequestError};
pub use self::request::{ClientRequest, ClientRequestBuilder};
pub use self::response::{ClientResponse, ResponseBody, JsonResponse, UrlEncoded};
pub use self::connector::{Connect, Connection, ClientConnector, ClientConnectorError};
pub(crate) use self::writer::HttpClientWriter;
pub(crate) use self::parser::{HttpResponseParser, HttpResponseParserError};
