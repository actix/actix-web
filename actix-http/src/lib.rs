//! HTTP primitives for the Actix ecosystem.
//!
//! ## Crate Features
//! | Feature          | Functionality                                         |
//! | ---------------- | ----------------------------------------------------- |
//! | `openssl`        | TLS support via [OpenSSL].                            |
//! | `rustls`         | TLS support via [rustls].                             |
//! | `compress`       | Payload compression support. (Deflate, Gzip & Brotli) |
//! | `trust-dns`      | Use [trust-dns] as the client DNS resolver.           |
//!
//! [OpenSSL]: https://crates.io/crates/openssl
//! [rustls]: https://crates.io/crates/rustls
//! [trust-dns]: https://crates.io/crates/trust-dns

#![deny(rust_2018_idioms, nonstandard_style)]
#![allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::new_without_default,
    clippy::borrow_interior_mutable_const
)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

#[macro_use]
extern crate log;

#[macro_use]
mod macros;

pub mod body;
mod builder;
pub mod client;
mod config;
#[cfg(feature = "compress")]
pub mod encoding;
mod extensions;
mod header;
mod helpers;
mod http_codes;
mod http_message;
mod message;
mod payload;
mod request;
mod response;
mod service;
mod time_parser;

pub mod error;
pub mod h1;
pub mod h2;
pub mod test;
pub mod ws;

pub use self::builder::HttpServiceBuilder;
pub use self::config::{KeepAlive, ServiceConfig};
pub use self::error::{Error, ResponseError, Result};
pub use self::extensions::Extensions;
pub use self::http_message::HttpMessage;
pub use self::message::{Message, RequestHead, RequestHeadType, ResponseHead};
pub use self::payload::{Payload, PayloadStream};
pub use self::request::Request;
pub use self::response::{Response, ResponseBuilder};
pub use self::service::HttpService;

pub mod http {
    //! Various HTTP related types.

    // re-exports
    pub use http::header::{HeaderName, HeaderValue};
    pub use http::uri::PathAndQuery;
    pub use http::{uri, Error, Uri};
    pub use http::{Method, StatusCode, Version};

    pub use crate::header::HeaderMap;

    /// A collection of HTTP headers and helpers.
    pub mod header {
        pub use crate::header::*;
    }
    pub use crate::header::ContentEncoding;
    pub use crate::message::ConnectionType;
}

/// A major HTTP protocol version.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Protocol {
    Http1,
    Http2,
    Http3,
}

type ConnectCallback<IO> = dyn Fn(&IO, &mut Extensions);

/// Container for data that extract with ConnectCallback.
///
/// # Implementation Details
/// Uses Option to reduce necessary allocations when merging with request extensions.
pub(crate) struct OnConnectData(Option<Extensions>);

impl Default for OnConnectData {
    fn default() -> Self {
        Self(None)
    }
}

impl OnConnectData {
    /// Construct by calling the on-connect callback with the underlying transport I/O.
    pub(crate) fn from_io<T>(
        io: &T,
        on_connect_ext: Option<&ConnectCallback<T>>,
    ) -> Self {
        let ext = on_connect_ext.map(|handler| {
            let mut extensions = Extensions::new();
            handler(io, &mut extensions);
            extensions
        });

        Self(ext)
    }

    /// Merge self into given request's extensions.
    #[inline]
    pub(crate) fn merge_into(&mut self, req: &mut Request) {
        if let Some(ref mut ext) = self.0 {
            req.head.extensions.get_mut().drain_from(ext);
        }
    }
}
