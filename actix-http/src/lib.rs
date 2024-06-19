//! HTTP types and services for the Actix ecosystem.
//!
//! ## Crate Features
//!
//! | Feature             | Functionality                               |
//! | ------------------- | ------------------------------------------- |
//! | `http2`             | HTTP/2 support via [h2].                    |
//! | `openssl`           | TLS support via [OpenSSL].                  |
//! | `rustls-0_20`       | TLS support via rustls 0.20.                |
//! | `rustls-0_21`       | TLS support via rustls 0.21.                |
//! | `rustls-0_22`       | TLS support via rustls 0.22.                |
//! | `rustls-0_23`       | TLS support via [rustls] 0.23.              |
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
    clippy::borrow_interior_mutable_const
)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub use http::{uri, uri::Uri, Method, StatusCode, Version};

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

#[allow(deprecated)]
pub use self::payload::PayloadStream;
#[cfg(feature = "__tls")]
pub use self::service::TlsAcceptorConfig;
pub use self::{
    builder::HttpServiceBuilder,
    config::ServiceConfig,
    error::Error,
    extensions::Extensions,
    header::ContentEncoding,
    http_message::HttpMessage,
    keep_alive::KeepAlive,
    message::{ConnectionType, Message},
    payload::{BoxedPayloadStream, Payload},
    requests::{Request, RequestHead, RequestHeadType},
    responses::{Response, ResponseBuilder, ResponseHead},
    service::HttpService,
};

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
