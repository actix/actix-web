use std::cell::UnsafeCell;
use std::fmt::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};
use std::{fmt, net};

use bytes::BytesMut;
use futures::{future, Future};
use log::error;
use time;
use tokio_timer::{sleep, Delay};

// "Sun, 06 Nov 1994 08:49:37 GMT".len()
const DATE_VALUE_LENGTH: usize = 29;

#[derive(Debug, PartialEq, Clone, Copy)]
/// Server keep-alive setting
pub enum KeepAlive {
    /// Keep alive in seconds
    Timeout(usize),
    /// Relay on OS to shutdown tcp connection
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
    timer: DateService,
}

impl Clone for ServiceConfig {
    fn clone(&self) -> Self {
        ServiceConfig(self.0.clone())
    }
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self::new(KeepAlive::Timeout(5), 0, 0)
    }
}

impl ServiceConfig {
    /// Create instance of `ServiceConfig`
    pub(crate) fn new(
        keep_alive: KeepAlive,
        client_timeout: u64,
        client_disconnect: u64,
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
            timer: DateService::with(Duration::from_millis(500)),
        }))
    }

    /// Create worker settings builder.
    pub fn build() -> ServiceConfigBuilder {
        ServiceConfigBuilder::new()
    }

    #[inline]
    /// Keep alive duration if configured.
    pub fn keep_alive(&self) -> Option<Duration> {
        self.0.keep_alive
    }

    #[inline]
    /// Return state of connection keep-alive funcitonality
    pub fn keep_alive_enabled(&self) -> bool {
        self.0.ka_enabled
    }

    #[inline]
    /// Client timeout for first request.
    pub fn client_timer(&self) -> Option<Delay> {
        let delay = self.0.client_timeout;
        if delay != 0 {
            Some(Delay::new(
                self.0.timer.now() + Duration::from_millis(delay),
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
            Some(Delay::new(self.0.timer.now() + ka))
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

    pub(crate) fn set_date(&self, dst: &mut BytesMut) {
        let mut buf: [u8; 39] = [0; 39];
        buf[..6].copy_from_slice(b"date: ");
        buf[6..35].copy_from_slice(&self.0.timer.date().bytes);
        buf[35..].copy_from_slice(b"\r\n\r\n");
        dst.extend_from_slice(&buf);
    }
}

/// A service config builder
///
/// This type can be used to construct an instance of `ServiceConfig` through a
/// builder-like pattern.
pub struct ServiceConfigBuilder {
    keep_alive: KeepAlive,
    client_timeout: u64,
    client_disconnect: u64,
    host: String,
    addr: net::SocketAddr,
    secure: bool,
}

impl ServiceConfigBuilder {
    /// Create instance of `ServiceConfigBuilder`
    pub fn new() -> ServiceConfigBuilder {
        ServiceConfigBuilder {
            keep_alive: KeepAlive::Timeout(5),
            client_timeout: 5000,
            client_disconnect: 0,
            secure: false,
            host: "localhost".to_owned(),
            addr: "127.0.0.1:8080".parse().unwrap(),
        }
    }

    /// Enable secure flag for current server.
    /// This flags also enables `client disconnect timeout`.
    ///
    /// By default this flag is set to false.
    pub fn secure(mut self) -> Self {
        self.secure = true;
        if self.client_disconnect == 0 {
            self.client_disconnect = 3000;
        }
        self
    }

    /// Set server keep-alive setting.
    ///
    /// By default keep alive is set to a 5 seconds.
    pub fn keep_alive<T: Into<KeepAlive>>(mut self, val: T) -> Self {
        self.keep_alive = val.into();
        self
    }

    /// Set server client timeout in milliseconds for first request.
    ///
    /// Defines a timeout for reading client request header. If a client does not transmit
    /// the entire set headers within this time, the request is terminated with
    /// the 408 (Request Time-out) error.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default client timeout is set to 5000 milliseconds.
    pub fn client_timeout(mut self, val: u64) -> Self {
        self.client_timeout = val;
        self
    }

    /// Set server connection disconnect timeout in milliseconds.
    ///
    /// Defines a timeout for disconnect connection. If a disconnect procedure does not complete
    /// within this time, the request get dropped. This timeout affects secure connections.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default disconnect timeout is set to 3000 milliseconds.
    pub fn client_disconnect(mut self, val: u64) -> Self {
        self.client_disconnect = val;
        self
    }

    /// Set server host name.
    ///
    /// Host name is used by application router aa a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    ///
    /// By default host name is set to a "localhost" value.
    pub fn server_hostname(mut self, val: &str) -> Self {
        self.host = val.to_owned();
        self
    }

    /// Set server ip address.
    ///
    /// Host name is used by application router aa a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    ///
    /// By default server address is set to a "127.0.0.1:8080"
    pub fn server_address<S: net::ToSocketAddrs>(mut self, addr: S) -> Self {
        match addr.to_socket_addrs() {
            Err(err) => error!("Can not convert to SocketAddr: {}", err),
            Ok(mut addrs) => {
                if let Some(addr) = addrs.next() {
                    self.addr = addr;
                }
            }
        }
        self
    }

    /// Finish service configuration and create `ServiceConfig` object.
    pub fn finish(self) -> ServiceConfig {
        ServiceConfig::new(self.keep_alive, self.client_timeout, self.client_disconnect)
    }
}

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
        write!(self, "{}", time::at_utc(time::get_time()).rfc822()).unwrap();
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
    interval: Duration,
    current: UnsafeCell<Option<(Date, Instant)>>,
}

impl DateServiceInner {
    fn new(interval: Duration) -> Self {
        DateServiceInner {
            interval,
            current: UnsafeCell::new(None),
        }
    }

    fn get_ref(&self) -> &Option<(Date, Instant)> {
        unsafe { &*self.current.get() }
    }

    fn reset(&self) {
        unsafe { (&mut *self.current.get()).take() };
    }

    fn update(&self) {
        let now = Instant::now();
        let date = Date::new();
        *(unsafe { &mut *self.current.get() }) = Some((date, now));
    }
}

impl DateService {
    fn with(resolution: Duration) -> Self {
        DateService(Rc::new(DateServiceInner::new(resolution)))
    }

    fn check_date(&self) {
        if self.0.get_ref().is_none() {
            self.0.update();

            // periodic date update
            let s = self.clone();
            tokio_current_thread::spawn(sleep(Duration::from_millis(500)).then(
                move |_| {
                    s.0.reset();
                    future::ok(())
                },
            ));
        }
    }

    fn now(&self) -> Instant {
        self.check_date();
        self.0.get_ref().as_ref().unwrap().1
    }

    fn date(&self) -> &Date {
        self.check_date();

        let item = self.0.get_ref().as_ref().unwrap();
        &item.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_rt::System;
    use futures::future;

    #[test]
    fn test_date_len() {
        assert_eq!(DATE_VALUE_LENGTH, "Sun, 06 Nov 1994 08:49:37 GMT".len());
    }

    #[test]
    fn test_date() {
        let mut rt = System::new("test");

        let _ = rt.block_on(future::lazy(|| {
            let settings = ServiceConfig::new(KeepAlive::Os, 0, 0);
            let mut buf1 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
            settings.set_date(&mut buf1);
            let mut buf2 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
            settings.set_date(&mut buf2);
            assert_eq!(buf1, buf2);
            future::ok::<_, ()>(())
        }));
    }
}
