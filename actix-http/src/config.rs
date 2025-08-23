use std::{
    net,
    rc::Rc,
    time::{Duration, Instant},
};

use bytes::BytesMut;

use crate::{date::DateService, KeepAlive};

/// HTTP service configuration.
#[derive(Debug, Clone)]
pub struct ServiceConfig(Rc<Inner>);

#[derive(Debug)]
struct Inner {
    keep_alive: KeepAlive,
    client_request_timeout: Duration,
    client_disconnect_timeout: Duration,
    secure: bool,
    local_addr: Option<std::net::SocketAddr>,
    date_service: DateService,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self::new(
            KeepAlive::default(),
            Duration::from_secs(5),
            Duration::ZERO,
            false,
            None,
        )
    }
}

impl ServiceConfig {
    /// Create instance of `ServiceConfig`.
    pub fn new(
        keep_alive: KeepAlive,
        client_request_timeout: Duration,
        client_disconnect_timeout: Duration,
        secure: bool,
        local_addr: Option<net::SocketAddr>,
    ) -> ServiceConfig {
        ServiceConfig(Rc::new(Inner {
            keep_alive: keep_alive.normalize(),
            client_request_timeout,
            client_disconnect_timeout,
            secure,
            local_addr,
            date_service: DateService::new(),
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
    pub fn local_addr(&self) -> Option<net::SocketAddr> {
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
