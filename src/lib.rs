//! Basic http primitives for actix-net framework.
#![allow(
    clippy::type_complexity,
    clippy::new_without_default,
    clippy::new_without_default_derive
)]

#[macro_use]
extern crate log;

pub mod body;
mod builder;
pub mod client;
mod config;
mod extensions;
mod header;
mod helpers;
mod httpcodes;
pub mod httpmessage;
mod message;
mod payload;
mod request;
mod response;
mod service;

pub mod error;
pub mod h1;
pub mod h2;
pub mod test;
pub mod ws;

pub use self::builder::HttpServiceBuilder;
pub use self::config::{KeepAlive, ServiceConfig};
pub use self::error::{Error, ResponseError, Result};
pub use self::extensions::Extensions;
pub use self::httpmessage::HttpMessage;
pub use self::message::{Head, Message, RequestHead, ResponseHead};
pub use self::payload::{Payload, PayloadStream};
pub use self::request::Request;
pub use self::response::{Response, ResponseBuilder};
pub use self::service::{HttpService, SendError, SendResponse};

pub mod http {
    //! Various HTTP related types

    // re-exports
    pub use http::header::{HeaderName, HeaderValue};
    pub use http::{Method, StatusCode, Version};

    #[doc(hidden)]
    pub use http::{uri, Error, HeaderMap, HttpTryFrom, Uri};

    #[doc(hidden)]
    pub use http::uri::PathAndQuery;

    #[cfg(feature = "cookies")]
    pub use cookie::{Cookie, CookieBuilder};

    /// Various http headers
    pub mod header {
        pub use crate::header::*;
    }
    pub use crate::header::ContentEncoding;
    pub use crate::message::ConnectionType;
}
