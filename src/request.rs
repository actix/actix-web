use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;

use http::{header, HeaderMap, Method, Uri, Version};

use extensions::Extensions;
use httpmessage::HttpMessage;
use payload::Payload;
use uri::Url;

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
    pub(crate) url: Url,
    pub(crate) flags: Cell<MessageFlags>,
    pub(crate) headers: HeaderMap,
    pub(crate) extensions: RefCell<Extensions>,
    pub(crate) payload: RefCell<Option<Payload>>,
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
    /// Create new Request instance
    pub fn new() -> Request {
        Request::with_pool(RequestPool::pool())
    }

    /// Create new Request instance with pool
    pub(crate) fn with_pool(pool: &'static RequestPool) -> Request {
        Request {
            inner: Rc::new(InnerRequest {
                pool,
                method: Method::GET,
                url: Url::default(),
                version: Version::HTTP_11,
                headers: HeaderMap::with_capacity(16),
                flags: Cell::new(MessageFlags::empty()),
                payload: RefCell::new(None),
                extensions: RefCell::new(Extensions::new()),
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
    pub fn url(&self) -> &Url {
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

    #[doc(hidden)]
    /// Note: this method should be called only as part of clone operation
    /// of wrapper type.
    pub fn clone_request(&self) -> Self {
        Request {
            inner: self.inner.clone(),
        }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        if Rc::strong_count(&self.inner) == 1 {
            self.inner.pool.release(self.inner.clone());
        }
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

/// Request's objects pool
pub(crate) struct RequestPool(RefCell<VecDeque<Rc<InnerRequest>>>);

thread_local!(static POOL: &'static RequestPool = RequestPool::create());

impl RequestPool {
    fn create() -> &'static RequestPool {
        let pool = RequestPool(RefCell::new(VecDeque::with_capacity(128)));
        Box::leak(Box::new(pool))
    }

    /// Get default request's pool
    pub fn pool() -> &'static RequestPool {
        POOL.with(|p| *p)
    }

    /// Get Request object
    #[inline]
    pub fn get(pool: &'static RequestPool) -> Request {
        if let Some(mut msg) = pool.0.borrow_mut().pop_front() {
            if let Some(r) = Rc::get_mut(&mut msg) {
                r.reset();
            }
            return Request { inner: msg };
        }
        Request::with_pool(pool)
    }

    #[inline]
    /// Release request instance
    pub(crate) fn release(&self, msg: Rc<InnerRequest>) {
        let v = &mut self.0.borrow_mut();
        if v.len() < 128 {
            v.push_front(msg);
        }
    }
}
