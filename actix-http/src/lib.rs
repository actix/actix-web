//! HTTP primitives for the Actix ecosystem.

#![deny(rust_2018_idioms)]
#![allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::new_without_default,
    clippy::borrow_interior_mutable_const
)]
#![allow(clippy::manual_strip)] // Allow this to keep MSRV(1.42).
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

#[macro_use]
extern crate log;

#[macro_use]
mod macros;

pub mod body;
mod builder;
pub mod client;
mod cloneable;
mod config;
#[cfg(feature = "compress")]
pub mod encoding;
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
mod time_parser;

pub use cookie;
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
pub use self::message::{Message, RequestHead, RequestHeadType, ResponseHead};
pub use self::payload::{Payload, PayloadStream};
pub use self::request::Request;
pub use self::response::{Response, ResponseBuilder};
pub use self::service::HttpService;

pub mod http {
    //! Various HTTP related types

    // re-exports
    pub use http::header::{HeaderName, HeaderValue};
    pub use http::uri::PathAndQuery;
    pub use http::{uri, Error, Uri};
    pub use http::{Method, StatusCode, Version};

    pub use crate::cookie::{Cookie, CookieBuilder};
    pub use crate::header::HeaderMap;

    /// Various http headers
    pub mod header {
        pub use crate::header::*;
    }
    pub use crate::header::ContentEncoding;
    pub use crate::message::ConnectionType;
}

/// Http protocol
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    Http1,
    Http2,
}

type ConnectCallback<IO> = dyn Fn(&IO, &mut Extensions);
