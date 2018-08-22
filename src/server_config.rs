//! Default server config
use std::sync::{atomic::AtomicUsize, Arc};

pub trait Config: Send + Clone + Default + 'static {
    fn fork(&self) -> Self;
}

#[derive(Clone, Default)]
pub struct ServerConfig {
    conn: ConnectionsConfig,
    ssl: SslConfig,
}

impl Config for ServerConfig {
    fn fork(&self) -> Self {
        ServerConfig {
            conn: self.conn.fork(),
            ssl: self.ssl.fork(),
        }
    }
}

impl AsRef<ConnectionsConfig> for ServerConfig {
    fn as_ref(&self) -> &ConnectionsConfig {
        &self.conn
    }
}

impl AsRef<SslConfig> for ServerConfig {
    fn as_ref(&self) -> &SslConfig {
        &self.ssl
    }
}

#[derive(Clone)]
pub struct ConnectionsConfig {
    max_connections: usize,
    num_connections: Arc<AtomicUsize>,
}

impl Default for ConnectionsConfig {
    fn default() -> Self {
        ConnectionsConfig {
            max_connections: 102_400,
            num_connections: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Config for ConnectionsConfig {
    fn fork(&self) -> Self {
        ConnectionsConfig {
            max_connections: self.max_connections,
            num_connections: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[derive(Clone)]
pub struct SslConfig {
    max_handshakes: usize,
    num: Arc<AtomicUsize>,
}

impl Default for SslConfig {
    fn default() -> Self {
        SslConfig {
            max_handshakes: 256,
            num: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Config for SslConfig {
    fn fork(&self) -> Self {
        SslConfig {
            max_handshakes: self.max_handshakes,
            num: Arc::new(AtomicUsize::new(0)),
        }
    }
}
