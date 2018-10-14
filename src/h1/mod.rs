//! HTTP/1 implementation
use actix_net::codec::Framed;

mod codec;
mod decoder;
mod dispatcher;
mod encoder;
mod service;

pub use self::codec::{Codec, InMessage, InMessageType, OutMessage};
pub use self::decoder::{PayloadDecoder, RequestDecoder};
pub use self::dispatcher::Dispatcher;
pub use self::service::{H1Service, H1ServiceHandler};

use request::Request;

/// H1 service response type
pub enum H1ServiceResult<T> {
    Disconnected,
    Shutdown(T),
    Unhandled(Request, Framed<T, Codec>),
}
