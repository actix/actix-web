use std::time::Duration;

// These values are taken from hyper/src/proto/h2/client.rs
const DEFAULT_H2_CONN_WINDOW: u32 = 1024 * 1024 * 2; // 2mb
const DEFAULT_H2_STREAM_WINDOW: u32 = 1024 * 1024; // 1mb

/// Connector configuration
#[derive(Clone)]
pub(crate) struct ConnectorConfig {
    pub(crate) timeout: Duration,
    pub(crate) conn_lifetime: Duration,
    pub(crate) conn_keep_alive: Duration,
    pub(crate) disconnect_timeout: Option<Duration>,
    pub(crate) limit: usize,
    pub(crate) conn_window_size: u32,
    pub(crate) stream_window_size: u32,
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(1),
            conn_lifetime: Duration::from_secs(75),
            conn_keep_alive: Duration::from_secs(15),
            disconnect_timeout: Some(Duration::from_millis(3000)),
            limit: 100,
            conn_window_size: DEFAULT_H2_CONN_WINDOW,
            stream_window_size: DEFAULT_H2_STREAM_WINDOW,
        }
    }
}

impl ConnectorConfig {
    pub(crate) fn no_disconnect_timeout(&self) -> Self {
        let mut res = self.clone();
        res.disconnect_timeout = None;
        res
    }
}
