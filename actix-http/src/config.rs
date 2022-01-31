use std::{
    cell::Cell,
    fmt::{self, Write},
    net,
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

use actix_rt::{task::JoinHandle, time::interval};
use bytes::BytesMut;

use crate::KeepAlive;

/// "Thu, 01 Jan 1970 00:00:00 GMT".len()
pub(crate) const DATE_VALUE_LENGTH: usize = 29;

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
    /// Create instance of `ServiceConfig`
    pub fn new(
        keep_alive: KeepAlive,
        client_request_timeout: Duration,
        client_disconnect_timeout: Duration,
        secure: bool,
        local_addr: Option<net::SocketAddr>,
    ) -> ServiceConfig {
        // zero timeout keep-alive maps to disabled
        let keep_alive = match keep_alive {
            KeepAlive::Timeout(Duration::ZERO) => KeepAlive::Disabled,
            ka => ka,
        };

        ServiceConfig(Rc::new(Inner {
            keep_alive,
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

        if timeout != Duration::ZERO {
            Some(self.now() + timeout)
        } else {
            None
        }
    }

    /// Creates a time object representing the deadline for the client to disconnect.
    pub fn client_disconnect_deadline(&self) -> Option<Instant> {
        let timeout = self.0.client_disconnect_timeout;

        if timeout != Duration::ZERO {
            Some(self.now() + timeout)
        } else {
            None
        }
    }

    pub(crate) fn now(&self) -> Instant {
        self.0.date_service.now()
    }

    pub(crate) fn set_date(&self, dst: &mut BytesMut, camel_case: bool) {
        let mut buf: [u8; 39] = [0; 39];

        buf[..6].copy_from_slice(if camel_case { b"Date: " } else { b"date: " });

        self.0
            .date_service
            .set_date(|date| buf[6..35].copy_from_slice(&date.bytes));

        buf[35..].copy_from_slice(b"\r\n\r\n");
        dst.extend_from_slice(&buf);
    }

    pub(crate) fn write_date_header(&self, dst: &mut BytesMut) {
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
        write!(self, "{}", httpdate::fmt_http_date(SystemTime::now())).unwrap();
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

impl DateService {
    fn new() -> Self {
        // shared date and timer for DateService and update async task.
        let current = Rc::new(Cell::new((Date::new(), Instant::now())));
        let current_clone = Rc::clone(&current);
        // spawn an async task sleep for 500 millis and update current date/timer in a loop.
        // handle is used to stop the task on DateService drop.
        let handle = actix_rt::spawn(async move {
            #[cfg(test)]
            let _notify = notify_on_drop::NotifyOnDrop::new();

            let mut interval = interval(Duration::from_millis(500));
            loop {
                let now = interval.tick().await;
                let date = Date::new();
                current_clone.set((date, now.into_std()));
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

impl fmt::Debug for DateService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DateService").finish_non_exhaustive()
    }
}

impl Drop for DateService {
    fn drop(&mut self) {
        // stop the timer update async task on drop.
        self.handle.abort();
    }
}

// TODO: move to a util module for testing all spawn handle drop style tasks.
/// Test Module for checking the drop state of certain async tasks that are spawned
/// with `actix_rt::spawn`
///
/// The target task must explicitly generate `NotifyOnDrop` when spawn the task
#[cfg(test)]
mod notify_on_drop {
    use std::cell::RefCell;

    thread_local! {
        static NOTIFY_DROPPED: RefCell<Option<bool>> = RefCell::new(None);
    }

    /// Check if the spawned task is dropped.
    ///
    /// # Panics
    /// Panics when there was no `NotifyOnDrop` instance on current thread.
    pub(crate) fn is_dropped() -> bool {
        NOTIFY_DROPPED.with(|bool| {
            bool.borrow()
                .expect("No NotifyOnDrop existed on current thread")
        })
    }

    pub(crate) struct NotifyOnDrop;

    impl NotifyOnDrop {
        /// # Panics
        /// Panics hen construct multiple instances on any given thread.
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

    use actix_rt::{
        task::yield_now,
        time::{sleep, sleep_until},
    };
    use memchr::memmem;

    #[actix_rt::test]
    async fn test_date_service_update() {
        let settings =
            ServiceConfig::new(KeepAlive::Os, Duration::ZERO, Duration::ZERO, false, None);

        yield_now().await;

        let mut buf1 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.set_date(&mut buf1, false);
        let now1 = settings.now();

        sleep_until((Instant::now() + Duration::from_secs(2)).into()).await;
        yield_now().await;

        let now2 = settings.now();
        let mut buf2 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.set_date(&mut buf2, false);

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
        settings.set_date(&mut buf1, false);

        let mut buf2 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.set_date(&mut buf2, false);

        assert_eq!(buf1, buf2);
    }

    #[actix_rt::test]
    async fn test_date_camel_case() {
        let settings = ServiceConfig::default();

        let mut buf = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.set_date(&mut buf, false);
        assert!(memmem::find(&buf, b"date:").is_some());

        let mut buf = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.set_date(&mut buf, true);
        assert!(memmem::find(&buf, b"Date:").is_some());
    }
}
