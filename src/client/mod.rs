mod parser;
mod response;

pub use self::response::ClientResponse;
pub use self::parser::{HttpResponseParser, HttpResponseParserError};
