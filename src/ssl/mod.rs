//! SSL Services
use std::sync::atomic::{AtomicUsize, Ordering};

use super::counter::Counter;

#[cfg(feature = "ssl")]
mod openssl;
#[cfg(feature = "ssl")]
pub use self::openssl::{OpensslAcceptor, OpensslConnector};

#[cfg(feature = "tls")]
mod nativetls;
#[cfg(feature = "tls")]
pub use self::nativetls::{NativeTlsAcceptor, TlsStream};

pub(crate) const MAX_CONN: AtomicUsize = AtomicUsize::new(256);

/// Sets the maximum per-worker concurrent ssl connection establish process.
///
/// All listeners will stop accepting connections when this limit is
/// reached. It can be used to limit the global SSL CPU usage.
///
/// By default max connections is set to a 256.
pub fn max_concurrent_ssl_connect(num: usize) {
    MAX_CONN.store(num, Ordering::Relaxed);
}

thread_local! {
    static MAX_CONN_COUNTER: Counter = Counter::new(MAX_CONN.load(Ordering::Relaxed));
}

// #[cfg(feature = "rust-tls")]
// mod rustls;
// #[cfg(feature = "rust-tls")]
// pub use self::rustls::RustlsAcceptor;
