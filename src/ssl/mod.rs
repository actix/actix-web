//! SSL Services
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "ssl")]
mod openssl;
#[cfg(feature = "ssl")]
pub use self::openssl::{OpensslAcceptor, OpensslConnector};

pub(crate) const MAX_CONN: AtomicUsize = AtomicUsize::new(0);

/// Set max concurrent ssl connect operation per thread
pub fn max_concurrent_ssl_connect(num: usize) {
    MAX_CONN.store(num, Ordering::Relaxed);
}

// #[cfg(feature = "tls")]
// mod nativetls;
// #[cfg(feature = "tls")]
// pub use self::nativetls::{NativeTlsAcceptor, TlsStream};

// #[cfg(feature = "rust-tls")]
// mod rustls;
// #[cfg(feature = "rust-tls")]
// pub use self::rustls::RustlsAcceptor;
