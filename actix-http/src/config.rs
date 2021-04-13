use std::cell::Cell;
use std::fmt::Write;
use std::rc::Rc;
use std::time::Duration;
use std::{fmt, net};

use actix_rt::{
    task::JoinHandle,
    time::{interval, sleep_until, Instant, Sleep},
};
use bytes::BytesMut;
use time::OffsetDateTime;

/// "Sun, 06 Nov 1994 08:49:37 GMT".len()
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
    date_service: DateService,
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
            date_service: DateService::new(),
        }))
    }

    /// Returns true if connection is secure (HTTPS)
    #[inline]
    pub fn secure(&self) -> bool {
        self.0.secure
    }

    /// Returns the local address that this server is bound to.
    #[inline]
    pub fn local_addr(&self) -> Option<net::SocketAddr> {
        self.0.local_addr
    }

    /// Keep alive duration if configured.
    #[inline]
    pub fn keep_alive(&self) -> Option<Duration> {
        self.0.keep_alive
    }

    /// Return state of connection keep-alive functionality
    #[inline]
    pub fn keep_alive_enabled(&self) -> bool {
        self.0.ka_enabled
    }

    /// Client timeout for first request.
    #[inline]
    pub fn client_timer(&self) -> Option<Sleep> {
        let delay_time = self.0.client_timeout;
        if delay_time != 0 {
            Some(sleep_until(self.now() + Duration::from_millis(delay_time)))
        } else {
            None
        }
    }

    /// Client timeout for first request.
    pub fn client_timer_expire(&self) -> Option<Instant> {
        let delay = self.0.client_timeout;
        if delay != 0 {
            Some(self.now() + Duration::from_millis(delay))
        } else {
            None
        }
    }

    /// Client disconnect timer
    pub fn client_disconnect_timer(&self) -> Option<Instant> {
        let delay = self.0.client_disconnect;
        if delay != 0 {
            Some(self.now() + Duration::from_millis(delay))
        } else {
            None
        }
    }

    #[inline]
    /// Return keep-alive timer delay is configured.
    pub fn keep_alive_timer(&self) -> Option<Sleep> {
        self.keep_alive().map(|ka| sleep_until(self.now() + ka))
    }

    /// Keep-alive expire time
    pub fn keep_alive_expire(&self) -> Option<Instant> {
        self.keep_alive().map(|ka| self.now() + ka)
    }

    #[inline]
    pub(crate) fn now(&self) -> Instant {
        self.0.date_service.now()
    }

    #[doc(hidden)]
    pub fn set_date(&self, dst: &mut BytesMut) {
        let mut buf: [u8; 39] = [0; 39];
        buf[..6].copy_from_slice(b"date: ");
        self.0
            .date_service
            .set_date(|date| buf[6..35].copy_from_slice(&date.bytes));
        buf[35..].copy_from_slice(b"\r\n\r\n");
        dst.extend_from_slice(&buf);
    }

    pub(crate) fn set_date_header(&self, dst: &mut BytesMut) {
        self.0
            .date_service
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

/// Service for update Date and Instant periodically at 500 millis interval.
struct DateService {
    current: Rc<Cell<(Date, Instant)>>,
    handle: JoinHandle<()>,
}

impl Drop for DateService {
    fn drop(&mut self) {
        // stop the timer update async task on drop.
        self.handle.abort();
    }
}

impl DateService {
    fn new() -> Self {
        // shared date and timer for DateService and update async task.
        let current = Rc::new(Cell::new((Date::new(), Instant::now())));
        let current_clone = Rc::clone(&current);
        // spawn an async task sleep for 500 milli and update current date/timer in a loop.
        // handle is used to stop the task on DateService drop.
        let handle = actix_rt::spawn(async move {
            #[cfg(test)]
            let _notify = notify_on_drop::NotifyOnDrop::new();

            let mut interval = interval(Duration::from_millis(500));
            loop {
                let now = interval.tick().await;
                let date = Date::new();
                current_clone.set((date, now));
            }
        });

        DateService { current, handle }
    }

    fn now(&self) -> Instant {
        self.current.get().1
    }

    fn set_date<F: FnMut(&Date)>(&self, mut f: F) {
        f(&self.current.get().0);
    }
}

// TODO: move to a util module for testing all spawn handle drop style tasks.
#[cfg(test)]
/// Test Module for checking the drop state of certain async tasks that are spawned
/// with `actix_rt::spawn`
///
/// The target task must explicitly generate `NotifyOnDrop` when spawn the task
mod notify_on_drop {
    use std::cell::RefCell;

    thread_local! {
        static NOTIFY_DROPPED: RefCell<Option<bool>> = RefCell::new(None);
    }

    /// Check if the spawned task is dropped.
    ///
    /// # Panic:
    ///
    /// When there was no `NotifyOnDrop` instance on current thread
    pub(crate) fn is_dropped() -> bool {
        NOTIFY_DROPPED.with(|bool| {
            bool.borrow()
                .expect("No NotifyOnDrop existed on current thread")
        })
    }

    pub(crate) struct NotifyOnDrop;

    impl NotifyOnDrop {
        /// # Panic:
        ///
        /// When construct multiple instances on any given thread.
        pub(crate) fn new() -> Self {
            NOTIFY_DROPPED.with(|bool| {
                let mut bool = bool.borrow_mut();
                if bool.is_some() {
                    panic!("NotifyOnDrop existed on current thread");
                } else {
                    *bool = Some(false);
                }
            });

            NotifyOnDrop
        }
    }

    impl Drop for NotifyOnDrop {
        fn drop(&mut self) {
            NOTIFY_DROPPED.with(|bool| {
                if let Some(b) = bool.borrow_mut().as_mut() {
                    *b = true;
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use actix_rt::task::yield_now;

    #[actix_rt::test]
    async fn test_date_service_update() {
        let settings = ServiceConfig::new(KeepAlive::Os, 0, 0, false, None);

        yield_now().await;

        let mut buf1 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.set_date(&mut buf1);
        let now1 = settings.now();

        sleep_until(Instant::now() + Duration::from_secs(2)).await;
        yield_now().await;

        let now2 = settings.now();
        let mut buf2 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.set_date(&mut buf2);

        assert_ne!(now1, now2);

        assert_ne!(buf1, buf2);

        drop(settings);
        assert!(notify_on_drop::is_dropped());
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
        assert_eq!(false, notify_on_drop::is_dropped());
        drop(clone2);
        assert_eq!(false, notify_on_drop::is_dropped());
        drop(clone3);
        assert_eq!(false, notify_on_drop::is_dropped());

        drop(service);
        assert!(notify_on_drop::is_dropped());
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
