use std::rc::Rc;
use std::sync::Arc;
use std::marker::PhantomData;
use std::collections::HashMap;

use serde_json;
use serde_json::error::Error as JsonError;
use serde::{Serialize, Deserialize};
use http::header::{self, HeaderValue};
use cookie::{CookieJar, Cookie, Key};
use futures::Future;
use futures::future::{FutureResult, ok as FutOk, err as FutErr};

use error::{Result, Error, ResponseError};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::{Middleware, Started, Response};

/// The helper trait to obtain your session data from a request.
///
/// ```rust
/// use actix_web::*;
/// use actix_web::middleware::RequestSession;
///
/// fn index(mut req: HttpRequest) -> Result<&'static str> {
///     // access session data
///     if let Some(count) = req.session().get::<i32>("counter")? {
///         req.session().set("counter", count+1)?;
///     } else {
///         req.session().set("counter", 1)?;
///     }
///
///     Ok("Welcome!")
/// }
/// # fn main() {}
/// ```
pub trait RequestSession {
    fn session(&mut self) -> Session;
}

impl<S> RequestSession for HttpRequest<S> {

    fn session(&mut self) -> Session {
        if let Some(s_impl) = self.extensions().get_mut::<Arc<SessionImplBox>>() {
            if let Some(s) = Arc::get_mut(s_impl) {
                return Session(s.0.as_mut())
            }
        }
        Session(unsafe{&mut DUMMY})
    }
}

/// The high-level interface you use to modify session data.
///
/// Session object could be obtained with
/// [`RequestSession::session`](trait.RequestSession.html#tymethod.session)
/// method. `RequestSession` trait is implemented for `HttpRequest`.
///
/// ```rust
/// use actix_web::*;
/// use actix_web::middleware::RequestSession;
///
/// fn index(mut req: HttpRequest) -> Result<&'static str> {
///     // access session data
///     if let Some(count) = req.session().get::<i32>("counter")? {
///         req.session().set("counter", count+1)?;
///     } else {
///         req.session().set("counter", 1)?;
///     }
///
///     Ok("Welcome!")
/// }
/// # fn main() {}
/// ```
pub struct Session<'a>(&'a mut SessionImpl);

impl<'a> Session<'a> {

    /// Get a `value` from the session.
    pub fn get<T: Deserialize<'a>>(&'a self, key: &str) -> Result<Option<T>> {
        if let Some(s) = self.0.get(key) {
            Ok(Some(serde_json::from_str(s)?))
        } else {
            Ok(None)
        }
    }

    /// Set a `value` from the session.
    pub fn set<T: Serialize>(&mut self, key: &str, value: T) -> Result<()> {
        self.0.set(key, serde_json::to_string(&value)?);
        Ok(())
    }

    /// Remove value from the session.
    pub fn remove(&'a mut self, key: &str) {
        self.0.remove(key)
    }

    /// Clear the session.
    pub fn clear(&'a mut self) {
        self.0.clear()
    }
}

struct SessionImplBox(Box<SessionImpl>);

#[doc(hidden)]
unsafe impl Send for SessionImplBox {}
#[doc(hidden)]
unsafe impl Sync for SessionImplBox {}

/// Session storage middleware
///
/// ```rust
/// # extern crate actix;
/// # extern crate actix_web;
/// # use actix_web::middleware::{SessionStorage, CookieSessionBackend};
/// use actix_web::*;
///
/// fn main() {
///    let app = Application::new().middleware(
///        SessionStorage::new(                      // <- create session middleware
///            CookieSessionBackend::build(&[0; 32]) // <- create cookie session backend
///               .secure(false)
///               .finish())
///    );
/// }
/// ```
pub struct SessionStorage<T, S>(T, PhantomData<S>);

impl<S, T: SessionBackend<S>> SessionStorage<T, S> {
    /// Create session storage
    pub fn new(backend: T) -> SessionStorage<T, S> {
        SessionStorage(backend, PhantomData)
    }
}

impl<S: 'static, T: SessionBackend<S>> Middleware<S> for SessionStorage<T, S> {

    fn start(&self, req: &mut HttpRequest<S>) -> Result<Started> {
        let mut req = req.clone();

        let fut = self.0.from_request(&mut req)
            .then(move |res| {
                match res {
                    Ok(sess) => {
                        req.extensions().insert(Arc::new(SessionImplBox(Box::new(sess))));
                        FutOk(None)
                    },
                    Err(err) => FutErr(err)
                }
            });
        Ok(Started::Future(Box::new(fut)))
    }

    fn response(&self, req: &mut HttpRequest<S>, resp: HttpResponse) -> Result<Response> {
        if let Some(s_box) = req.extensions().remove::<Arc<SessionImplBox>>() {
            s_box.0.write(resp)
        } else {
            Ok(Response::Done(resp))
        }
    }
}

/// A simple key-value storage interface that is internally used by `Session`.
#[doc(hidden)]
pub trait SessionImpl: 'static {

    fn get(&self, key: &str) -> Option<&str>;

    fn set(&mut self, key: &str, value: String);

    fn remove(&mut self, key: &str);

    fn clear(&mut self);

    /// Write session to storage backend.
    fn write(&self, resp: HttpResponse) -> Result<Response>;
}

/// Session's storage backend trait definition.
#[doc(hidden)]
pub trait SessionBackend<S>: Sized + 'static {
    type Session: SessionImpl;
    type ReadFuture: Future<Item=Self::Session, Error=Error>;

    /// Parse the session from request and load data from a storage backend.
    fn from_request(&self, request: &mut HttpRequest<S>) -> Self::ReadFuture;
}

/// Dummy session impl, does not do anything
struct DummySessionImpl;

static mut DUMMY: DummySessionImpl = DummySessionImpl;

impl SessionImpl for DummySessionImpl {

    fn get(&self, _: &str) -> Option<&str> { None }
    fn set(&mut self, _: &str, _: String) {}
    fn remove(&mut self, _: &str) {}
    fn clear(&mut self) {}
    fn write(&self, resp: HttpResponse) -> Result<Response> {
        Ok(Response::Done(resp))
    }
}

/// Session that uses signed cookies as session storage
pub struct CookieSession {
    changed: bool,
    state: HashMap<String, String>,
    inner: Rc<CookieSessionInner>,
}

/// Errors that can occur during handling cookie session
#[derive(Fail, Debug)]
pub enum CookieSessionError {
    /// Size of the serialized session is greater than 4000 bytes.
    #[fail(display="Size of the serialized session is greater than 4000 bytes.")]
    Overflow,
    /// Fail to serialize session.
    #[fail(display="Fail to serialize session")]
    Serialize(JsonError),
}

impl ResponseError for CookieSessionError {}

impl SessionImpl for CookieSession {

    fn get(&self, key: &str) -> Option<&str> {
        if let Some(s) = self.state.get(key) {
            Some(s)
        } else {
            None
        }
    }

    fn set(&mut self, key: &str, value: String) {
        self.changed = true;
        self.state.insert(key.to_owned(), value);
    }

    fn remove(&mut self, key: &str) {
        self.changed = true;
        self.state.remove(key);
    }

    fn clear(&mut self) {
        self.changed = true;
        self.state.clear()
    }

    fn write(&self, mut resp: HttpResponse) -> Result<Response> {
        if self.changed {
            let _ = self.inner.set_cookie(&mut resp, &self.state);
        }
        Ok(Response::Done(resp))
    }
}

struct CookieSessionInner {
    key: Key,
    name: String,
    path: String,
    domain: Option<String>,
    secure: bool,
}

impl CookieSessionInner {

    fn new(key: &[u8]) -> CookieSessionInner {
        CookieSessionInner {
            key: Key::from_master(key),
            name: "actix-session".to_owned(),
            path: "/".to_owned(),
            domain: None,
            secure: true }
    }

    fn set_cookie(&self, resp: &mut HttpResponse, state: &HashMap<String, String>) -> Result<()> {
        let value = serde_json::to_string(&state)
            .map_err(CookieSessionError::Serialize)?;
        if value.len() > 4064 {
            return Err(CookieSessionError::Overflow.into())
        }

        let mut cookie = Cookie::new(self.name.clone(), value);
        cookie.set_path(self.path.clone());
        cookie.set_secure(self.secure);
        cookie.set_http_only(true);

        if let Some(ref domain) = self.domain {
            cookie.set_domain(domain.clone());
        }

        let mut jar = CookieJar::new();
        jar.signed(&self.key).add(cookie);

        for cookie in jar.delta() {
            let val = HeaderValue::from_str(&cookie.to_string())?;
            resp.headers_mut().append(header::SET_COOKIE, val);
        }

        Ok(())
    }

    fn load<S>(&self, req: &mut HttpRequest<S>) -> HashMap<String, String> {
        if let Ok(cookies) = req.cookies() {
            for cookie in cookies {
                if cookie.name() == self.name {
                    let mut jar = CookieJar::new();
                    jar.add_original(cookie.clone());
                    if let Some(cookie) = jar.signed(&self.key).get(&self.name) {
                        if let Ok(val) = serde_json::from_str(cookie.value()) {
                            return val;
                        }
                    }
                }
            }
        }
        HashMap::new()
    }
}

/// Use signed cookies as session storage.
///
/// `CookieSessionBackend` creates sessions which are limited to storing
/// fewer than 4000 bytes of data (as the payload must fit into a single cookie).
/// Internal server error get generated if session contains more than 4000 bytes.
///
/// You need to pass a random value to the constructor of `CookieSessionBackend`.
/// This is private key for cookie session, When this value is changed, all session data is lost.
///
/// Note that whatever you write into your session is visible by the user (but not modifiable).
///
/// Constructor panics if key length is less than 32 bytes.
pub struct CookieSessionBackend(Rc<CookieSessionInner>);

impl CookieSessionBackend {

    /// Construct new `CookieSessionBackend` instance.
    ///
    /// Panics if key length is less than 32 bytes.
    pub fn new(key: &[u8]) -> CookieSessionBackend {
        CookieSessionBackend(
            Rc::new(CookieSessionInner::new(key)))
    }

    /// Creates a new `CookieSessionBackendBuilder` instance from the given key.
    ///
    /// Panics if key length is less than 32 bytes.
    ///
    /// # Example
    ///
    /// ```
    /// use actix_web::middleware::CookieSessionBackend;
    ///
    /// let backend = CookieSessionBackend::build(&[0; 32]).finish();
    /// ```
    pub fn build(key: &[u8]) -> CookieSessionBackendBuilder {
        CookieSessionBackendBuilder::new(key)
    }
}

impl<S> SessionBackend<S> for CookieSessionBackend {

    type Session = CookieSession;
    type ReadFuture = FutureResult<CookieSession, Error>;

    fn from_request(&self, req: &mut HttpRequest<S>) -> Self::ReadFuture {
        let state = self.0.load(req);
        FutOk(
            CookieSession {
                changed: false,
                inner: Rc::clone(&self.0),
                state,
            })
    }
}

/// Structure that follows the builder pattern for building `CookieSessionBackend` structs.
///
/// To construct a backend:
///
///   1. Call [`CookieSessionBackend::build`](struct.CookieSessionBackend.html#method.build) to start building.
///   2. Use any of the builder methods to set fields in the backend.
///   3. Call [finish](#method.finish) to retrieve the constructed backend.
///
/// # Example
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::middleware::CookieSessionBackend;
///
/// # fn main() {
/// let backend: CookieSessionBackend = CookieSessionBackend::build(&[0; 32])
///     .domain("www.rust-lang.org")
///     .name("actix_session")
///     .path("/")
///     .secure(true)
///     .finish();
/// # }
/// ```
pub struct CookieSessionBackendBuilder(CookieSessionInner);

impl CookieSessionBackendBuilder {
    pub fn new(key: &[u8]) -> CookieSessionBackendBuilder {
        CookieSessionBackendBuilder(
            CookieSessionInner::new(key))
    }

    /// Sets the `path` field in the session cookie being built.
    pub fn path<S: Into<String>>(mut self, value: S) -> CookieSessionBackendBuilder {
        self.0.path = value.into();
        self
    }

    /// Sets the `name` field in the session cookie being built.
    pub fn name<S: Into<String>>(mut self, value: S) -> CookieSessionBackendBuilder {
        self.0.name = value.into();
        self
    }

    /// Sets the `domain` field in the session cookie being built.
    pub fn domain<S: Into<String>>(mut self, value: S) -> CookieSessionBackendBuilder {
        self.0.domain = Some(value.into());
        self
    }

    /// Sets the `secure` field in the session cookie being built.
    pub fn secure(mut self, value: bool) -> CookieSessionBackendBuilder {
        self.0.secure = value;
        self
    }

    /// Finishes building and returns the built `CookieSessionBackend`.
    pub fn finish(self) -> CookieSessionBackend {
        CookieSessionBackend(Rc::new(self.0))
    }
}
