use std::{fmt, mem, net};
use std::fmt::Write;
use std::rc::Rc;
use std::sync::Arc;
use std::cell::{Cell, RefCell, RefMut, UnsafeCell};
use time;
use bytes::BytesMut;
use http::StatusCode;
use futures_cpupool::{Builder, CpuPool};

use super::helpers;
use super::KeepAlive;
use super::channel::Node;
use super::shared::{SharedBytes, SharedBytesPool};
use body::Body;
use httpresponse::{HttpResponse, HttpResponsePool, HttpResponseBuilder};

/// Various server settings
#[derive(Clone)]
pub struct ServerSettings {
    addr: Option<net::SocketAddr>,
    secure: bool,
    host: String,
    cpu_pool: Arc<InnerCpuPool>,
    responses: Rc<UnsafeCell<HttpResponsePool>>,
}

unsafe impl Sync for ServerSettings {}
unsafe impl Send for ServerSettings {}

struct InnerCpuPool {
    cpu_pool: UnsafeCell<Option<CpuPool>>,
}

impl fmt::Debug for InnerCpuPool {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "CpuPool")
    }
}

impl InnerCpuPool {
    fn new() -> Self {
        InnerCpuPool {
            cpu_pool: UnsafeCell::new(None),
        }
    }
    fn cpu_pool(&self) -> &CpuPool {
        unsafe {
            let val = &mut *self.cpu_pool.get();
            if val.is_none() {
                *val = Some(Builder::new().create());
            }
            val.as_ref().unwrap()
        }
    }
}

unsafe impl Sync for InnerCpuPool {}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings {
            addr: None,
            secure: false,
            host: "localhost:8080".to_owned(),
            responses: HttpResponsePool::pool(),
            cpu_pool: Arc::new(InnerCpuPool::new()),
        }
    }
}

impl ServerSettings {
    /// Crate server settings instance
    pub(crate) fn new(addr: Option<net::SocketAddr>, host: &Option<String>, secure: bool)
                      -> ServerSettings
    {
        let host = if let Some(ref host) = *host {
            host.clone()
        } else if let Some(ref addr) = addr {
            format!("{}", addr)
        } else {
            "localhost".to_owned()
        };
        let cpu_pool = Arc::new(InnerCpuPool::new());
        let responses = HttpResponsePool::pool();
        ServerSettings { addr, secure, host, cpu_pool, responses }
    }

    /// Returns the socket address of the local half of this TCP connection
    pub fn local_addr(&self) -> Option<net::SocketAddr> {
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
        self.cpu_pool.cpu_pool()
    }

    #[inline]
    pub(crate) fn get_response(&self, status: StatusCode, body: Body) -> HttpResponse {
        HttpResponsePool::get_response(&self.responses, status, body)
    }

    #[inline]
    pub(crate) fn get_response_builder(&self, status: StatusCode) -> HttpResponseBuilder {
        HttpResponsePool::get_builder(&self.responses, status)
    }
}


// "Sun, 06 Nov 1994 08:49:37 GMT".len()
const DATE_VALUE_LENGTH: usize = 29;

pub(crate) struct WorkerSettings<H> {
    h: RefCell<Vec<H>>,
    keep_alive: u64,
    ka_enabled: bool,
    bytes: Rc<SharedBytesPool>,
    messages: Rc<helpers::SharedMessagePool>,
    channels: Cell<usize>,
    node: Box<Node<()>>,
    date: UnsafeCell<Date>,
}

impl<H> WorkerSettings<H> {
    pub(crate) fn new(h: Vec<H>, keep_alive: KeepAlive) -> WorkerSettings<H> {
        let (keep_alive, ka_enabled) = match keep_alive {
            KeepAlive::Timeout(val) => (val as u64, true),
            KeepAlive::Os | KeepAlive::Tcp(_) => (0, true),
            KeepAlive::Disabled => (0, false),
        };

        WorkerSettings {
            keep_alive, ka_enabled,
            h: RefCell::new(h),
            bytes: Rc::new(SharedBytesPool::new()),
            messages: Rc::new(helpers::SharedMessagePool::new()),
            channels: Cell::new(0),
            node: Box::new(Node::head()),
            date: UnsafeCell::new(Date::new()),
        }
    }

    pub fn num_channels(&self) -> usize {
        self.channels.get()
    }

    pub fn head(&self) -> &Node<()> {
        &self.node
    }

    pub fn handlers(&self) -> RefMut<Vec<H>> {
        self.h.borrow_mut()
    }

    pub fn keep_alive(&self) -> u64 {
        self.keep_alive
    }

    pub fn keep_alive_enabled(&self) -> bool {
        self.ka_enabled
    }

    pub fn get_shared_bytes(&self) -> SharedBytes {
        SharedBytes::new(self.bytes.get_bytes(), Rc::clone(&self.bytes))
    }

    pub fn get_http_message(&self) -> helpers::SharedHttpInnerMessage {
        helpers::SharedHttpInnerMessage::new(self.messages.get(), Rc::clone(&self.messages))
    }

    pub fn add_channel(&self) {
        self.channels.set(self.channels.get() + 1);
    }

    pub fn remove_channel(&self) {
        let num = self.channels.get();
        if num > 0 {
            self.channels.set(num-1);
        } else {
            error!("Number of removed channels is bigger than added channel. Bug in actix-web");
        }
    }

    pub fn update_date(&self) {
        unsafe{&mut *self.date.get()}.update();
    }

    pub fn set_date(&self, dst: &mut BytesMut) {
        let mut buf: [u8; 39] = unsafe { mem::uninitialized() };
        buf[..6].copy_from_slice(b"date: ");
        buf[6..35].copy_from_slice(&(unsafe{&*self.date.get()}.bytes));
        buf[35..].copy_from_slice(b"\r\n\r\n");
        dst.extend_from_slice(&buf);
    }

    pub fn set_date_simple(&self, dst: &mut BytesMut) {
        dst.extend_from_slice(&(unsafe{&*self.date.get()}.bytes));
    }
}

struct Date {
    bytes: [u8; DATE_VALUE_LENGTH],
    pos: usize,
}

impl Date {
    fn new() -> Date {
        let mut date = Date{bytes: [0; DATE_VALUE_LENGTH], pos: 0};
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_date_len() {
        assert_eq!(DATE_VALUE_LENGTH, "Sun, 06 Nov 1994 08:49:37 GMT".len());
    }

    #[test]
    fn test_date() {
        let settings = WorkerSettings::<()>::new(Vec::new(), KeepAlive::Os);
        let mut buf1 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.set_date(&mut buf1);
        let mut buf2 = BytesMut::with_capacity(DATE_VALUE_LENGTH + 10);
        settings.set_date(&mut buf2);
        assert_eq!(buf1, buf2);
    }
}
