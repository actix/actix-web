use std::{
    net::SocketAddr,
    rc::Rc,
    time::{Duration, Instant},
};

use bytes::BytesMut;

use crate::{date::DateService, KeepAlive};

/// A builder for creating a [`ServiceConfig`]
#[derive(Default, Debug)]
pub struct ServiceConfigBuilder {
    inner: Inner,
}

impl ServiceConfigBuilder {
    /// Creates a new, default, [`ServiceConfigBuilder`]
    ///
    /// It uses the following default values:
    ///
    /// - [`KeepAlive::default`] for the connection keep-alive setting
    /// - 5 seconds for the client request timeout
    /// - 0 seconds for the client shutdown timeout
    /// - secure value of `false`
    /// - [`None`] for the local address setting
    /// - Allow for half closed HTTP/1 connections
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the `secure` attribute for this configuration
    pub fn secure(mut self, secure: bool) -> Self {
        self.inner.secure = secure;
        self
    }

    /// Sets the local address for this configuration
    pub fn local_addr(mut self, local_addr: Option<SocketAddr>) -> Self {
        self.inner.local_addr = local_addr;
        self
    }

    /// Sets connection keep-alive setting
    pub fn keep_alive(mut self, keep_alive: KeepAlive) -> Self {
        self.inner.keep_alive = keep_alive;
        self
    }

    /// Sets the timeout for the client to finish sending the head of its first request
    pub fn client_request_timeout(mut self, timeout: Duration) -> Self {
        self.inner.client_request_timeout = timeout;
        self
    }

    /// Sets the timeout for cleanly disconnecting from the client after connection shutdown has
    /// started
    pub fn client_disconnect_timeout(mut self, timeout: Duration) -> Self {
        self.inner.client_disconnect_timeout = timeout;
        self
    }

    /// Sets whether HTTP/1 connections should support half-closures.
    ///
    /// Clients can choose to shutdown their writer-side of the connection after completing their
    /// request and while waiting for the server response. Setting this to `false` will cause the
    /// server to abort the connection handling as soon as it detects an EOF from the client
    pub fn h1_allow_half_closed(mut self, allow: bool) -> Self {
        self.inner.h1_allow_half_closed = allow;
        self
    }

    /// Builds a [`ServiceConfig`] from this [`ServiceConfigBuilder`] instance
    pub fn build(self) -> ServiceConfig {
        ServiceConfig(Rc::new(self.inner))
    }
}

/// HTTP service configuration.
#[derive(Debug, Clone, Default)]
pub struct ServiceConfig(Rc<Inner>);

#[derive(Debug)]
struct Inner {
    keep_alive: KeepAlive,
    client_request_timeout: Duration,
    client_disconnect_timeout: Duration,
    secure: bool,
    local_addr: Option<SocketAddr>,
    date_service: DateService,
    h1_allow_half_closed: bool,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            keep_alive: KeepAlive::default(),
            client_request_timeout: Duration::from_secs(5),
            client_disconnect_timeout: Duration::ZERO,
            secure: false,
            local_addr: None,
            date_service: DateService::new(),
            h1_allow_half_closed: true,
        }
    }
}

impl ServiceConfig {
    /// Create instance of `ServiceConfig`.
    pub fn new(
        keep_alive: KeepAlive,
        client_request_timeout: Duration,
        client_disconnect_timeout: Duration,
        secure: bool,
        local_addr: Option<SocketAddr>,
    ) -> ServiceConfig {
        ServiceConfig(Rc::new(Inner {
            keep_alive: keep_alive.normalize(),
            client_request_timeout,
            client_disconnect_timeout,
            secure,
            local_addr,
            date_service: DateService::new(),
            h1_allow_half_closed: true,
        }))
    }

    /// Returns `true` if connection is secure (i.e., using TLS / HTTPS).
    #[inline]
    pub fn secure(&self) -> bool {
        self.0.secure
    }

    /// Returns the local address that this server is bound to.
    ///
    /// Returns `None` for connections via UDS (Unix Domain Socket).
    #[inline]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.0.local_addr
    }

    /// Connection keep-alive setting.
    #[inline]
    pub fn keep_alive(&self) -> KeepAlive {
        self.0.keep_alive
    }

    /// Creates a time object representing the deadline for this connection's keep-alive period, if
    /// enabled.
    ///
    /// When [`KeepAlive::Os`] or [`KeepAlive::Disabled`] is set, this will return `None`.
    pub fn keep_alive_deadline(&self) -> Option<Instant> {
        match self.keep_alive() {
            KeepAlive::Timeout(dur) => Some(self.now() + dur),
            KeepAlive::Os => None,
            KeepAlive::Disabled => None,
        }
    }

    /// Creates a time object representing the deadline for the client to finish sending the head of
    /// its first request.
    ///
    /// Returns `None` if this `ServiceConfig was` constructed with `client_request_timeout: 0`.
    pub fn client_request_deadline(&self) -> Option<Instant> {
        let timeout = self.0.client_request_timeout;
        (timeout != Duration::ZERO).then(|| self.now() + timeout)
    }

    /// Creates a time object representing the deadline for the client to disconnect.
    pub fn client_disconnect_deadline(&self) -> Option<Instant> {
        let timeout = self.0.client_disconnect_timeout;
        (timeout != Duration::ZERO).then(|| self.now() + timeout)
    }

    /// Whether HTTP/1 connections should support half-closures.
    ///
    /// Clients can choose to shutdown their writer-side of the connection after completing their
    /// request and while waiting for the server response. If this configuration is `false`, the
    /// server will abort the connection handling as soon as it detects an EOF from the client
    pub fn h1_allow_half_closed(&self) -> bool {
        self.0.h1_allow_half_closed
    }

    pub(crate) fn now(&self) -> Instant {
        self.0.date_service.now()
    }

    /// Writes date header to `dst` buffer.
    ///
    /// Low-level method that utilizes the built-in efficient date service, requiring fewer syscalls
    /// than normal. Note that a CRLF (`\r\n`) is included in what is written.
    #[doc(hidden)]
    pub fn write_date_header(&self, dst: &mut BytesMut, camel_case: bool) {
        let mut buf: [u8; 37] = [0; 37];

        buf[..6].copy_from_slice(if camel_case { b"Date: " } else { b"date: " });

        self.0
            .date_service
            .with_date(|date| buf[6..35].copy_from_slice(&date.bytes));

        buf[35..].copy_from_slice(b"\r\n");
        dst.extend_from_slice(&buf);
    }

    #[allow(unused)] // used with `http2` feature flag
    pub(crate) fn write_date_header_value(&self, dst: &mut BytesMut) {
        self.0
            .date_service
            .with_date(|date| dst.extend_from_slice(&date.bytes));
    }
}

#[cfg(test)]
mod tests {
    use actix_rt::{
        task::yield_now,
        time::{sleep, sleep_until},
    };
    use memchr::memmem;

    use super::*;
    use crate::{date::DATE_VALUE_LENGTH, notify_on_drop};

    #[actix_rt::test]
    async fn test_date_service_update() {
        let settings =
            ServiceConfig::new(KeepAlive::Os, Duration::ZERO, Duration::ZERO, false, None);

        yield_now().await;

        let mut buf1 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.write_date_header(&mut buf1, false);
        let now1 = settings.now();

        sleep_until((Instant::now() + Duration::from_secs(2)).into()).await;
        yield_now().await;

        let now2 = settings.now();
        let mut buf2 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.write_date_header(&mut buf2, false);

        assert_ne!(now1, now2);

        assert_ne!(buf1, buf2);

        drop(settings);

        // Ensure the task will drop eventually
        let mut times = 0;
        while !notify_on_drop::is_dropped() {
            sleep(Duration::from_millis(100)).await;
            times += 1;
            assert!(times < 10, "Timeout waiting for task drop");
        }
    }

    #[actix_rt::test]
    async fn test_date_service_drop() {
        let service = Rc::new(DateService::new());

        // yield so date service have a chance to register the spawned timer update task.
        yield_now().await;

        let clone1 = service.clone();
        let clone2 = service.clone();
        let clone3 = service.clone();

        drop(clone1);
        assert!(!notify_on_drop::is_dropped());
        drop(clone2);
        assert!(!notify_on_drop::is_dropped());
        drop(clone3);
        assert!(!notify_on_drop::is_dropped());

        drop(service);

        // Ensure the task will drop eventually
        let mut times = 0;
        while !notify_on_drop::is_dropped() {
            sleep(Duration::from_millis(100)).await;
            times += 1;
            assert!(times < 10, "Timeout waiting for task drop");
        }
    }

    #[test]
    fn test_date_len() {
        assert_eq!(DATE_VALUE_LENGTH, "Sun, 06 Nov 1994 08:49:37 GMT".len());
    }

    #[actix_rt::test]
    async fn test_date() {
        let settings = ServiceConfig::default();

        let mut buf1 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.write_date_header(&mut buf1, false);

        let mut buf2 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.write_date_header(&mut buf2, false);

        assert_eq!(buf1, buf2);
    }

    #[actix_rt::test]
    async fn test_date_camel_case() {
        let settings = ServiceConfig::default();

        let mut buf = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.write_date_header(&mut buf, false);
        assert!(memmem::find(&buf, b"date:").is_some());

        let mut buf = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.write_date_header(&mut buf, true);
        assert!(memmem::find(&buf, b"Date:").is_some());
    }
}
