//! HTTP primitives for the Actix ecosystem.
//!
//! ## Crate Features
//! | Feature             | Functionality                               |
//! | ------------------- | ------------------------------------------- |
//! | `http2`             | HTTP/2 support via [h2].                    |
//! | `openssl`           | TLS support via [OpenSSL].                  |
//! | `rustls`            | TLS support via [rustls].                   |
//! | `compress-brotli`   | Payload compression support: Brotli.        |
//! | `compress-gzip`     | Payload compression support: Deflate, Gzip. |
//! | `compress-zstd`     | Payload compression support: Zstd.          |
//! | `trust-dns`         | Use [trust-dns] as the client DNS resolver. |
//!
//! [h2]: https://crates.io/crates/h2
//! [OpenSSL]: https://crates.io/crates/openssl
//! [rustls]: https://crates.io/crates/rustls
//! [trust-dns]: https://crates.io/crates/trust-dns

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
#![allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::borrow_interior_mutable_const,
    clippy::uninlined_format_args
)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub use ::http::{uri, uri::Uri};
pub use ::http::{Method, StatusCode, Version};

pub mod body;
mod builder;
mod config;
mod date;
#[cfg(feature = "__compress")]
pub mod encoding;
pub mod error;
mod extensions;
pub mod h1;
#[cfg(feature = "http2")]
pub mod h2;
pub mod header;
mod helpers;
mod http_message;
mod keep_alive;
mod message;
#[cfg(test)]
mod notify_on_drop;
mod payload;
mod requests;
mod responses;
mod service;
pub mod test;
#[cfg(feature = "ws")]
pub mod ws;

pub use self::builder::HttpServiceBuilder;
pub use self::config::ServiceConfig;
pub use self::error::Error;
pub use self::extensions::Extensions;
pub use self::header::ContentEncoding;
pub use self::http_message::HttpMessage;
pub use self::keep_alive::KeepAlive;
pub use self::message::ConnectionType;
pub use self::message::Message;
#[allow(deprecated)]
pub use self::payload::{BoxedPayloadStream, Payload, PayloadStream};
pub use self::requests::{Request, RequestHead, RequestHeadType};
pub use self::responses::{Response, ResponseBuilder, ResponseHead};
pub use self::service::HttpService;
#[cfg(any(feature = "openssl", feature = "rustls"))]
pub use self::service::TlsAcceptorConfig;

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
#[derive(Default)]
pub(crate) struct OnConnectData(Option<Extensions>);

impl OnConnectData {
    /// Construct by calling the on-connect callback with the underlying transport I/O.
    pub(crate) fn from_io<T>(io: &T, on_connect_ext: Option<&ConnectCallback<T>>) -> Self {
        let ext = on_connect_ext.map(|handler| {
            let mut extensions = Extensions::default();
            handler(io, &mut extensions);
            extensions
        });

        Self(ext)
    }
}
