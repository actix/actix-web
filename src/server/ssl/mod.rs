#[cfg(feature = "alpn")]
mod openssl;
#[cfg(feature = "alpn")]
pub use self::openssl::OpensslAcceptor;

#[cfg(feature = "tls")]
mod nativetls;
#[cfg(feature = "tls")]
pub use self::nativetls::{TlsStream, NativeTlsAcceptor};

#[cfg(feature = "rust-tls")]
mod rustls;
#[cfg(feature = "rust-tls")]
pub use self::rustls::RustlsAcceptor;
