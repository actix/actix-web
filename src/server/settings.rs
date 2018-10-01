use std::cell::{RefCell, RefMut, UnsafeCell};
use std::collections::VecDeque;
use std::fmt::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};
use std::{env, fmt, net};

use bytes::BytesMut;
use futures::{future, Future};
use futures_cpupool::CpuPool;
use http::StatusCode;
use lazycell::LazyCell;
use parking_lot::Mutex;
use time;
use tokio_current_thread::spawn;
use tokio_timer::{sleep, Delay};

use super::channel::Node;
use super::message::{Request, RequestPool};
use super::KeepAlive;
use body::Body;
use httpresponse::{HttpResponse, HttpResponseBuilder, HttpResponsePool};

/// Env variable for default cpu pool size
const ENV_CPU_POOL_VAR: &str = "ACTIX_CPU_POOL";

lazy_static! {
    pub(crate) static ref DEFAULT_CPUPOOL: Mutex<CpuPool> = {
        let default = match env::var(ENV_CPU_POOL_VAR) {
            Ok(val) => {
                if let Ok(val) = val.parse() {
                    val
                } else {
                    error!("Can not parse ACTIX_CPU_POOL value");
                    20
                }
            }
            Err(_) => 20,
        };
        Mutex::new(CpuPool::new(default))
    };
}

/// Various server settings
pub struct ServerSettings {
    addr: net::SocketAddr,
    secure: bool,
    host: String,
    cpu_pool: LazyCell<CpuPool>,
    responses: &'static HttpResponsePool,
}

impl Clone for ServerSettings {
    fn clone(&self) -> Self {
        ServerSettings {
            addr: self.addr,
            secure: self.secure,
            host: self.host.clone(),
            cpu_pool: LazyCell::new(),
            responses: HttpResponsePool::get_pool(),
        }
    }
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings {
            addr: "127.0.0.1:8080".parse().unwrap(),
            secure: false,
            host: "localhost:8080".to_owned(),
            responses: HttpResponsePool::get_pool(),
            cpu_pool: LazyCell::new(),
        }
    }
}

impl ServerSettings {
    /// Crate server settings instance
    pub fn new(addr: net::SocketAddr, host: &str, secure: bool) -> ServerSettings {
        let host = host.to_owned();
        let cpu_pool = LazyCell::new();
        let responses = HttpResponsePool::get_pool();
        ServerSettings {
            addr,
            secure,
            host,
            cpu_pool,
            responses,
        }
    }

    /// Returns the socket address of the local half of this TCP connection
    pub fn local_addr(&self) -> net::SocketAddr {
        self.addr
    }

    /// Returns true if connection is secure(https)
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// Returns host header value
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns default `CpuPool` for server
    pub fn cpu_pool(&self) -> &CpuPool {
        self.cpu_pool.borrow_with(|| DEFAULT_CPUPOOL.lock().clone())
    }

    #[inline]
    pub(crate) fn get_response(&self, status: StatusCode, body: Body) -> HttpResponse {
        HttpResponsePool::get_response(&self.responses, status, body)
    }

    #[inline]
    pub(crate) fn get_response_builder(
        &self, status: StatusCode,
    ) -> HttpResponseBuilder {
        HttpResponsePool::get_builder(&self.responses, status)
    }
}

// "Sun, 06 Nov 1994 08:49:37 GMT".len()
const DATE_VALUE_LENGTH: usize = 29;

pub struct WorkerSettings<H>(Rc<Inner<H>>);

struct Inner<H> {
    handler: H,
    keep_alive: Option<Duration>,
    client_timeout: u64,
    ka_enabled: bool,
    bytes: Rc<SharedBytesPool>,
    messages: &'static RequestPool,
    node: RefCell<Node<()>>,
    date: UnsafeCell<(bool, Date)>,
}

impl<H> Clone for WorkerSettings<H> {
    fn clone(&self) -> Self {
        WorkerSettings(self.0.clone())
    }
}

impl<H> WorkerSettings<H> {
    /// Create instance of `WorkerSettings`
    pub fn new(
        handler: H, keep_alive: KeepAlive, client_timeout: u64, settings: ServerSettings,
    ) -> WorkerSettings<H> {
        let (keep_alive, ka_enabled) = match keep_alive {
            KeepAlive::Timeout(val) => (val as u64, true),
            KeepAlive::Os | KeepAlive::Tcp(_) => (0, true),
            KeepAlive::Disabled => (0, false),
        };
        let keep_alive = if ka_enabled && keep_alive > 0 {
            Some(Duration::from_secs(keep_alive))
        } else {
            None
        };

        WorkerSettings(Rc::new(Inner {
            handler,
            keep_alive,
            ka_enabled,
            client_timeout,
            bytes: Rc::new(SharedBytesPool::new()),
            messages: RequestPool::pool(settings),
            node: RefCell::new(Node::head()),
            date: UnsafeCell::new((false, Date::new())),
        }))
    }

    pub(crate) fn head(&self) -> RefMut<Node<()>> {
        self.0.node.borrow_mut()
    }

    pub(crate) fn handler(&self) -> &H {
        &self.0.handler
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

    pub(crate) fn get_bytes(&self) -> BytesMut {
        self.0.bytes.get_bytes()
    }

    pub(crate) fn release_bytes(&self, bytes: BytesMut) {
        self.0.bytes.release_bytes(bytes)
    }

    pub(crate) fn get_request(&self) -> Request {
        RequestPool::get(self.0.messages)
    }

    fn update_date(&self) {
        // Unsafe: WorkerSetting is !Sync and !Send
        unsafe { (&mut *self.0.date.get()).0 = false };
    }
}

impl<H: 'static> WorkerSettings<H> {
    #[inline]
    /// Client timeout for first request.
    pub fn client_timer(&self) -> Option<Delay> {
        let delay = self.0.client_timeout;
        if delay != 0 {
            Some(Delay::new(self.now() + Duration::from_millis(delay)))
        } else {
            None
        }
    }

    #[inline]
    /// Return keep-alive timer delay is configured.
    pub fn keep_alive_timer(&self) -> Option<Delay> {
        if let Some(ka) = self.0.keep_alive {
            Some(Delay::new(self.now() + ka))
        } else {
            None
        }
    }

    /// Keep-alive expire time
    pub fn keep_alive_expire(&self) -> Option<Instant> {
        if let Some(ka) = self.0.keep_alive {
            Some(self.now() + ka)
        } else {
            None
        }
    }

    pub(crate) fn set_date(&self, dst: &mut BytesMut, full: bool) {
        // Unsafe: WorkerSetting is !Sync and !Send
        let date_bytes = unsafe {
            let date = &mut (*self.0.date.get());
            if !date.0 {
                date.1.update();
                date.0 = true;

                // periodic date update
                let s = self.clone();
                spawn(sleep(Duration::from_secs(1)).then(move |_| {
                    s.update_date();
                    future::ok(())
                }));
            }
            &date.1.bytes
        };
        if full {
            let mut buf: [u8; 39] = [0; 39];
            buf[..6].copy_from_slice(b"date: ");
            buf[6..35].copy_from_slice(date_bytes);
            buf[35..].copy_from_slice(b"\r\n\r\n");
            dst.extend_from_slice(&buf);
        } else {
            dst.extend_from_slice(date_bytes);
        }
    }

    #[inline]
    pub(crate) fn now(&self) -> Instant {
        unsafe {
            let date = &mut (*self.0.date.get());
            if !date.0 {
                date.1.update();
                date.0 = true;

                // periodic date update
                let s = self.clone();
                spawn(sleep(Duration::from_secs(1)).then(move |_| {
                    s.update_date();
                    future::ok(())
                }));
            }
            date.1.current
        }
    }
}

struct Date {
    current: Instant,
    bytes: [u8; DATE_VALUE_LENGTH],
    pos: usize,
}

impl Date {
    fn new() -> Date {
        let mut date = Date {
            current: Instant::now(),
            bytes: [0; DATE_VALUE_LENGTH],
            pos: 0,
        };
        date.update();
        date
    }
    fn update(&mut self) {
        self.pos = 0;
        self.current = Instant::now();
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

#[derive(Debug)]
pub(crate) struct SharedBytesPool(RefCell<VecDeque<BytesMut>>);

impl SharedBytesPool {
    pub fn new() -> SharedBytesPool {
        SharedBytesPool(RefCell::new(VecDeque::with_capacity(128)))
    }

    pub fn get_bytes(&self) -> BytesMut {
        if let Some(bytes) = self.0.borrow_mut().pop_front() {
            bytes
        } else {
            BytesMut::new()
        }
    }

    pub fn release_bytes(&self, mut bytes: BytesMut) {
        let v = &mut self.0.borrow_mut();
        if v.len() < 128 {
            bytes.clear();
            v.push_front(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future;
    use tokio::runtime::current_thread;

    #[test]
    fn test_date_len() {
        assert_eq!(DATE_VALUE_LENGTH, "Sun, 06 Nov 1994 08:49:37 GMT".len());
    }

    #[test]
    fn test_date() {
        let mut rt = current_thread::Runtime::new().unwrap();

        let _ = rt.block_on(future::lazy(|| {
            let settings = WorkerSettings::<()>::new(
                (),
                KeepAlive::Os,
                0,
                ServerSettings::default(),
            );
            let mut buf1 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
            settings.set_date(&mut buf1, true);
            let mut buf2 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
            settings.set_date(&mut buf2, true);
            assert_eq!(buf1, buf2);
            future::ok::<_, ()>(())
        }));
    }
}
