use std::{net::IpAddr, time::Duration};

const DEFAULT_H2_CONN_WINDOW: u32 = 1024 * 1024 * 2; // 2MB
const DEFAULT_H2_STREAM_WINDOW: u32 = 1024 * 1024; // 1MB

/// Connect configuration
#[derive(Clone, Hash, Eq, PartialEq)]
pub struct ConnectConfig {
    pub(crate) timeout: Duration,
    pub(crate) handshake_timeout: Duration,
    pub(crate) conn_lifetime: Duration,
    pub(crate) conn_keep_alive: Duration,
    pub(crate) conn_window_size: u32,
    pub(crate) stream_window_size: u32,
    pub(crate) local_address: Option<IpAddr>,
}

/// Connector configuration
#[derive(Clone)]
pub struct ConnectorConfig {
    pub(crate) default_connect_config: ConnectConfig,
    pub(crate) disconnect_timeout: Option<Duration>,
    pub(crate) limit: usize,
}

impl Default for ConnectConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            handshake_timeout: Duration::from_secs(5),
            conn_lifetime: Duration::from_secs(75),
            conn_keep_alive: Duration::from_secs(15),
            conn_window_size: DEFAULT_H2_CONN_WINDOW,
            stream_window_size: DEFAULT_H2_STREAM_WINDOW,
            local_address: None,
        }
    }
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            default_connect_config: ConnectConfig::default(),
            disconnect_timeout: Some(Duration::from_millis(3000)),
            limit: 100,
        }
    }
}

impl ConnectorConfig {
    pub fn no_disconnect_timeout(&self) -> Self {
        let mut res = self.clone();
        res.disconnect_timeout = None;
        res
    }
}

impl ConnectConfig {
    /// Sets TCP connection timeout.
    ///
    /// This is the max time allowed to connect to remote host, including DNS name resolution.
    ///
    /// By default, the timeout is 5 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets TLS handshake timeout.
    ///
    /// This is the max time allowed to perform the TLS handshake with remote host after TCP
    /// connection is established.
    ///
    /// By default, the timeout is 5 seconds.
    pub fn handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// Sets the initial window size (in bytes) for HTTP/2 stream-level flow control for received
    /// data.
    ///
    /// The default value is 65,535 and is good for APIs, but not for big objects.
    pub fn initial_window_size(mut self, size: u32) -> Self {
        self.stream_window_size = size;
        self
    }

    /// Sets the initial window size (in bytes) for HTTP/2 connection-level flow control for
    /// received data.
    ///
    /// The default value is 65,535 and is good for APIs, but not for big objects.
    pub fn initial_connection_window_size(mut self, size: u32) -> Self {
        self.conn_window_size = size;
        self
    }

    /// Set keep-alive period for opened connection.
    ///
    /// Keep-alive period is the period between connection usage. If
    /// the delay between repeated usages of the same connection
    /// exceeds this period, the connection is closed.
    /// Default keep-alive period is 15 seconds.
    pub fn conn_keep_alive(mut self, dur: Duration) -> Self {
        self.conn_keep_alive = dur;
        self
    }

    /// Set max lifetime period for connection.
    ///
    /// Connection lifetime is max lifetime of any opened connection
    /// until it is closed regardless of keep-alive period.
    /// Default lifetime period is 75 seconds.
    pub fn conn_lifetime(mut self, dur: Duration) -> Self {
        self.conn_lifetime = dur;
        self
    }

    /// Set local IP Address the connector would use for establishing connection.
    pub fn local_address(mut self, addr: IpAddr) -> Self {
        self.local_address = Some(addr);
        self
    }
}
