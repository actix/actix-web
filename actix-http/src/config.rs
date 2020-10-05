use std::cell::Cell;
use std::fmt::Write;
use std::rc::Rc;
use std::time::Duration;
use std::{fmt, net};

use actix_rt::time::{delay_for, delay_until, Delay, Instant};
use bytes::BytesMut;
use futures_util::{future, FutureExt};
use time::OffsetDateTime;

// "Sun, 06 Nov 1994 08:49:37 GMT".len()
const DATE_VALUE_LENGTH: usize = 29;

#[derive(Debug, PartialEq, Clone, Copy)]
/// Server keep-alive setting
pub enum KeepAlive {
    /// Keep alive in seconds
    Timeout(usize),
    /// Rely on OS to shutdown tcp connection
    Os,
    /// Disabled
    Disabled,
}

impl From<usize> for KeepAlive {
    fn from(keepalive: usize) -> Self {
        KeepAlive::Timeout(keepalive)
    }
}

impl From<Option<usize>> for KeepAlive {
    fn from(keepalive: Option<usize>) -> Self {
        if let Some(keepalive) = keepalive {
            KeepAlive::Timeout(keepalive)
        } else {
            KeepAlive::Disabled
        }
    }
}

/// Http service configuration
pub struct ServiceConfig(Rc<Inner>);

struct Inner {
    keep_alive: Option<Duration>,
    client_timeout: u64,
    client_disconnect: u64,
    ka_enabled: bool,
    secure: bool,
    local_addr: Option<std::net::SocketAddr>,
    timer: DateService,
}

impl Clone for ServiceConfig {
    fn clone(&self) -> Self {
        ServiceConfig(self.0.clone())
    }
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self::new(KeepAlive::Timeout(5), 0, 0, false, None)
    }
}

impl ServiceConfig {
    /// Create instance of `ServiceConfig`
    pub fn new(
        keep_alive: KeepAlive,
        client_timeout: u64,
        client_disconnect: u64,
        secure: bool,
        local_addr: Option<net::SocketAddr>,
    ) -> ServiceConfig {
        let (keep_alive, ka_enabled) = match keep_alive {
            KeepAlive::Timeout(val) => (val as u64, true),
            KeepAlive::Os => (0, true),
            KeepAlive::Disabled => (0, false),
        };
        let keep_alive = if ka_enabled && keep_alive > 0 {
            Some(Duration::from_secs(keep_alive))
        } else {
            None
        };

        ServiceConfig(Rc::new(Inner {
            keep_alive,
            ka_enabled,
            client_timeout,
            client_disconnect,
            secure,
            local_addr,
            timer: DateService::new(),
        }))
    }

    #[inline]
    /// Returns true if connection is secure(https)
    pub fn secure(&self) -> bool {
        self.0.secure
    }

    #[inline]
    /// Returns the local address that this server is bound to.
    pub fn local_addr(&self) -> Option<net::SocketAddr> {
        self.0.local_addr
    }

    #[inline]
    /// Keep alive duration if configured.
    pub fn keep_alive(&self) -> Option<Duration> {
        self.0.keep_alive
    }

    #[inline]
    /// Return state of connection keep-alive functionality
    pub fn keep_alive_enabled(&self) -> bool {
        self.0.ka_enabled
    }

    #[inline]
    /// Client timeout for first request.
    pub fn client_timer(&self) -> Option<Delay> {
        let delay_time = self.0.client_timeout;
        if delay_time != 0 {
            Some(delay_until(
                self.0.timer.now() + Duration::from_millis(delay_time),
            ))
        } else {
            None
        }
    }

    /// Client timeout for first request.
    pub fn client_timer_expire(&self) -> Option<Instant> {
        let delay = self.0.client_timeout;
        if delay != 0 {
            Some(self.0.timer.now() + Duration::from_millis(delay))
        } else {
            None
        }
    }

    /// Client disconnect timer
    pub fn client_disconnect_timer(&self) -> Option<Instant> {
        let delay = self.0.client_disconnect;
        if delay != 0 {
            Some(self.0.timer.now() + Duration::from_millis(delay))
        } else {
            None
        }
    }

    #[inline]
    /// Return keep-alive timer delay is configured.
    pub fn keep_alive_timer(&self) -> Option<Delay> {
        if let Some(ka) = self.0.keep_alive {
            Some(delay_until(self.0.timer.now() + ka))
        } else {
            None
        }
    }

    /// Keep-alive expire time
    pub fn keep_alive_expire(&self) -> Option<Instant> {
        if let Some(ka) = self.0.keep_alive {
            Some(self.0.timer.now() + ka)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) fn now(&self) -> Instant {
        self.0.timer.now()
    }

    #[doc(hidden)]
    pub fn set_date(&self, dst: &mut BytesMut) {
        let mut buf: [u8; 39] = [0; 39];
        buf[..6].copy_from_slice(b"date: ");
        self.0
            .timer
            .set_date(|date| buf[6..35].copy_from_slice(&date.bytes));
        buf[35..].copy_from_slice(b"\r\n\r\n");
        dst.extend_from_slice(&buf);
    }

    pub(crate) fn set_date_header(&self, dst: &mut BytesMut) {
        self.0
            .timer
            .set_date(|date| dst.extend_from_slice(&date.bytes));
    }
}

#[derive(Copy, Clone)]
struct Date {
    bytes: [u8; DATE_VALUE_LENGTH],
    pos: usize,
}

impl Date {
    fn new() -> Date {
        let mut date = Date {
            bytes: [0; DATE_VALUE_LENGTH],
            pos: 0,
        };
        date.update();
        date
    }

    fn update(&mut self) {
        self.pos = 0;
        write!(
            self,
            "{}",
            OffsetDateTime::now_utc().format("%a, %d %b %Y %H:%M:%S GMT")
        )
        .unwrap();
    }
}

impl fmt::Write for Date {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let len = s.len();
        self.bytes[self.pos..self.pos + len].copy_from_slice(s.as_bytes());
        self.pos += len;
        Ok(())
    }
}

#[derive(Clone)]
struct DateService(Rc<DateServiceInner>);

struct DateServiceInner {
    current: Cell<Option<(Date, Instant)>>,
}

impl DateServiceInner {
    fn new() -> Self {
        DateServiceInner {
            current: Cell::new(None),
        }
    }

    fn reset(&self) {
        self.current.take();
    }

    fn update(&self) {
        let now = Instant::now();
        let date = Date::new();
        self.current.set(Some((date, now)));
    }
}

impl DateService {
    fn new() -> Self {
        DateService(Rc::new(DateServiceInner::new()))
    }

    fn check_date(&self) {
        if self.0.current.get().is_none() {
            self.0.update();

            // periodic date update
            let s = self.clone();
            actix_rt::spawn(delay_for(Duration::from_millis(500)).then(move |_| {
                s.0.reset();
                future::ready(())
            }));
        }
    }

    fn now(&self) -> Instant {
        self.check_date();
        self.0.current.get().unwrap().1
    }

    fn set_date<F: FnMut(&Date)>(&self, mut f: F) {
        self.check_date();
        f(&self.0.current.get().unwrap().0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test modifying the date from within the closure
    // passed to `set_date`
    #[test]
    fn test_evil_date() {
        let service = DateService::new();
        // Make sure that `check_date` doesn't try to spawn a task
        service.0.update();
        service.set_date(|_| service.0.reset());
    }

    #[test]
    fn test_date_len() {
        assert_eq!(DATE_VALUE_LENGTH, "Sun, 06 Nov 1994 08:49:37 GMT".len());
    }

    #[actix_rt::test]
    async fn test_date() {
        let settings = ServiceConfig::new(KeepAlive::Os, 0, 0, false, None);
        let mut buf1 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.set_date(&mut buf1);
        let mut buf2 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.set_date(&mut buf2);
        assert_eq!(buf1, buf2);
    }
}
