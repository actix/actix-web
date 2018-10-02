use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::VecDeque;
use std::fmt;
use std::net::SocketAddr;
use std::rc::Rc;

use http::{header, HeaderMap, Method, Uri, Version};

use extensions::Extensions;
use httpmessage::HttpMessage;
use info::ConnectionInfo;
use payload::Payload;
use server::ServerSettings;
use uri::Url as InnerUrl;

bitflags! {
    pub(crate) struct MessageFlags: u8 {
        const KEEPALIVE = 0b0000_0001;
        const CONN_INFO = 0b0000_0010;
    }
}

/// Request's context
pub struct Request {
    pub(crate) inner: Rc<InnerRequest>,
}

pub(crate) struct InnerRequest {
    pub(crate) version: Version,
    pub(crate) method: Method,
    pub(crate) url: InnerUrl,
    pub(crate) flags: Cell<MessageFlags>,
    pub(crate) headers: HeaderMap,
    pub(crate) extensions: RefCell<Extensions>,
    pub(crate) addr: Option<SocketAddr>,
    pub(crate) info: RefCell<ConnectionInfo>,
    pub(crate) payload: RefCell<Option<Payload>>,
    pub(crate) settings: ServerSettings,
    pub(crate) stream_extensions: Option<Rc<Extensions>>,
    pool: &'static RequestPool,
}

impl InnerRequest {
    #[inline]
    /// Reset request instance
    pub fn reset(&mut self) {
        self.headers.clear();
        self.extensions.borrow_mut().clear();
        self.flags.set(MessageFlags::empty());
        *self.payload.borrow_mut() = None;
    }
}

impl HttpMessage for Request {
    type Stream = Payload;

    fn headers(&self) -> &HeaderMap {
        &self.inner.headers
    }

    #[inline]
    fn payload(&self) -> Payload {
        if let Some(payload) = self.inner.payload.borrow_mut().take() {
            payload
        } else {
            Payload::empty()
        }
    }
}

impl Request {
    /// Create new RequestContext instance
    pub(crate) fn new(pool: &'static RequestPool, settings: ServerSettings) -> Request {
        Request {
            inner: Rc::new(InnerRequest {
                pool,
                settings,
                method: Method::GET,
                url: InnerUrl::default(),
                version: Version::HTTP_11,
                headers: HeaderMap::with_capacity(16),
                flags: Cell::new(MessageFlags::empty()),
                addr: None,
                info: RefCell::new(ConnectionInfo::default()),
                payload: RefCell::new(None),
                extensions: RefCell::new(Extensions::new()),
                stream_extensions: None,
            }),
        }
    }

    #[inline]
    pub(crate) fn inner(&self) -> &InnerRequest {
        self.inner.as_ref()
    }

    #[inline]
    pub(crate) fn inner_mut(&mut self) -> &mut InnerRequest {
        Rc::get_mut(&mut self.inner).expect("Multiple copies exist")
    }

    #[inline]
    pub(crate) fn url(&self) -> &InnerUrl {
        &self.inner().url
    }

    /// Read the Request Uri.
    #[inline]
    pub fn uri(&self) -> &Uri {
        self.inner().url.uri()
    }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.inner().method
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.inner().version
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        self.inner().url.path()
    }

    #[inline]
    /// Returns Request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.inner().headers
    }

    #[inline]
    /// Returns mutable Request's headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.inner_mut().headers
    }

    /// Peer socket address
    ///
    /// Peer address is actual socket address, if proxy is used in front of
    /// actix http server, then peer address would be address of this proxy.
    ///
    /// To get client connection information `connection_info()` method should
    /// be used.
    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.inner().addr
    }

    /// Checks if a connection should be kept alive.
    #[inline]
    pub fn keep_alive(&self) -> bool {
        self.inner().flags.get().contains(MessageFlags::KEEPALIVE)
    }

    /// Request extensions
    #[inline]
    pub fn extensions(&self) -> Ref<Extensions> {
        self.inner().extensions.borrow()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<Extensions> {
        self.inner().extensions.borrow_mut()
    }

    /// Check if request requires connection upgrade
    pub fn upgrade(&self) -> bool {
        if let Some(conn) = self.inner().headers.get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                return s.to_lowercase().contains("upgrade");
            }
        }
        self.inner().method == Method::CONNECT
    }

    /// Get *ConnectionInfo* for the correct request.
    pub fn connection_info(&self) -> Ref<ConnectionInfo> {
        if self.inner().flags.get().contains(MessageFlags::CONN_INFO) {
            self.inner().info.borrow()
        } else {
            let mut flags = self.inner().flags.get();
            flags.insert(MessageFlags::CONN_INFO);
            self.inner().flags.set(flags);
            self.inner().info.borrow_mut().update(self);
            self.inner().info.borrow()
        }
    }

    /// Io stream extensions
    #[inline]
    pub fn stream_extensions(&self) -> Option<&Extensions> {
        self.inner().stream_extensions.as_ref().map(|e| e.as_ref())
    }

    /// Server settings
    #[inline]
    pub fn server_settings(&self) -> &ServerSettings {
        &self.inner().settings
    }

    pub(crate) fn clone(&self) -> Self {
        Request {
            inner: self.inner.clone(),
        }
    }

    pub(crate) fn release(self) {
        let mut inner = self.inner;
        if let Some(r) = Rc::get_mut(&mut inner) {
            r.reset();
        } else {
            return;
        }
        inner.pool.release(inner);
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "\nRequest {:?} {}:{}",
            self.version(),
            self.method(),
            self.path()
        )?;
        if let Some(q) = self.uri().query().as_ref() {
            writeln!(f, "  query: ?{:?}", q)?;
        }
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers().iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

pub(crate) struct RequestPool(
    RefCell<VecDeque<Rc<InnerRequest>>>,
    RefCell<ServerSettings>,
);

thread_local!(static POOL: &'static RequestPool = RequestPool::create());

impl RequestPool {
    fn create() -> &'static RequestPool {
        let pool = RequestPool(
            RefCell::new(VecDeque::with_capacity(128)),
            RefCell::new(ServerSettings::default()),
        );
        Box::leak(Box::new(pool))
    }

    pub fn pool(settings: ServerSettings) -> &'static RequestPool {
        POOL.with(|p| {
            *p.1.borrow_mut() = settings;
            *p
        })
    }

    #[inline]
    pub fn get(pool: &'static RequestPool) -> Request {
        if let Some(msg) = pool.0.borrow_mut().pop_front() {
            Request { inner: msg }
        } else {
            Request::new(pool, pool.1.borrow().clone())
        }
    }

    #[inline]
    /// Release request instance
    pub fn release(&self, msg: Rc<InnerRequest>) {
        let v = &mut self.0.borrow_mut();
        if v.len() < 128 {
            v.push_front(msg);
        }
    }
}
