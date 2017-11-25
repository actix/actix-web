#![allow(dead_code, unused_imports, unused_variables)]

use std::any::Any;
use std::rc::Rc;
use std::sync::Arc;
use serde_json;
use serde::{Serialize, Deserialize};
use http::header::{self, HeaderValue};
use cookie::{CookieJar, Cookie, Key};
use futures::Future;
use futures::future::{FutureResult, ok as FutOk, err as FutErr};

use error::{Result, Error, ErrorResponse};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middlewares::{Middleware, Started, Response};

/// The helper trait to obtain your session data from a request.
pub trait RequestSession {
    fn session(&mut self) -> Session;
}

impl RequestSession for HttpRequest {

    fn session(&mut self) -> Session {
        if let Some(s_impl) = self.extensions().get_mut::<Arc<SessionImplBox>>() {
            if let Some(s) = Arc::get_mut(s_impl) {
                return Session(s.0.as_mut())
            }
        }
        //Session(&mut DUMMY)
        unreachable!()
    }
}

/// The high-level interface you use to modify session data.
///
/// Session object could be obtained with
/// [`RequestSession::session`](trait.RequestSession.html#tymethod.session)
/// method. `RequestSession` trait is implemented for `HttpRequest`.
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
    pub fn set<T: Serialize>(&'a mut self, key: &str, value: T) -> Result<()> {
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
pub struct SessionStorage<T>(T);

impl<T: SessionBackend> SessionStorage<T> {
    /// Create session storage
    pub fn new(backend: T) -> SessionStorage<T> {
        SessionStorage(backend)
    }
}

impl<T: SessionBackend> Middleware for SessionStorage<T> {

    fn start(&self, mut req: HttpRequest) -> Started {
        let fut = self.0.from_request(&mut req)
            .then(|res| {
                match res {
                    Ok(sess) => {
                        req.extensions().insert(Arc::new(SessionImplBox(Box::new(sess))));
                        let resp: Option<HttpResponse> = None;
                        FutOk((req, resp))
                    },
                    Err(err) => FutErr(err)
                }
            });
        Started::Future(Box::new(fut))
    }

    fn response(&self, req: &mut HttpRequest, resp: HttpResponse) -> Response {
        if let Some(s_box) = req.extensions().remove::<Arc<SessionImplBox>>() {
            s_box.0.write(resp)
        } else {
            Response::Response(resp)
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
    fn write(&self, resp: HttpResponse) -> Response;
}

/// Session's storage backend trait definition.
#[doc(hidden)]
pub trait SessionBackend: Sized + 'static {
    type Session: SessionImpl;
    type ReadFuture: Future<Item=Self::Session, Error=Error>;

    /// Parse the session from request and load data from a storage backend.
    fn from_request(&self, request: &mut HttpRequest) -> Self::ReadFuture;
}

/// Dummy session impl, does not do anything
struct DummySessionImpl;

static DUMMY: DummySessionImpl = DummySessionImpl;

impl SessionImpl for DummySessionImpl {

    fn get(&self, key: &str) -> Option<&str> {
        None
    }
    fn set(&mut self, key: &str, value: String) {}
    fn remove(&mut self, key: &str) {}
    fn clear(&mut self) {}
    fn write(&self, resp: HttpResponse) -> Response {
        Response::Response(resp)
    }
}

/// Session that uses signed cookies as session storage
pub struct CookieSession {
    jar: CookieJar,
    key: Rc<Key>,
}

impl SessionImpl for CookieSession {

    fn get(&self, key: &str) -> Option<&str> {
        unimplemented!()
    }

    fn set(&mut self, key: &str, value: String) {
        unimplemented!()
    }

    fn remove(&mut self, key: &str) {
        unimplemented!()
    }

    fn clear(&mut self) {
        let cookies: Vec<_> = self.jar.iter().map(|c| c.clone()).collect();
        for cookie in cookies {
            self.jar.remove(cookie);
        }
    }

    fn write(&self, mut resp: HttpResponse) -> Response {
        for cookie in self.jar.delta() {
            match HeaderValue::from_str(&cookie.to_string()) {
                Err(err) => return Response::Response(err.error_response()),
                Ok(val) => resp.headers.append(header::SET_COOKIE, val),
            };
        }
        Response::Response(resp)
    }
}

/// Use signed cookies as session storage.
///
/// You need to pass a random value to the constructor of `CookieSessionBackend`.
/// This is private key for cookie session, When this value is changed, all session data is lost.
///
/// Note that whatever you write into your session is visible by the user (but not modifiable).
///
/// Constructor panics if key length is less than 32 bytes.
pub struct CookieSessionBackend {
    key: Rc<Key>,
}

impl CookieSessionBackend {

    /// Construct new `CookieSessionBackend` instance.
    ///
    /// Panics if key length is less than 32 bytes.
    pub fn new(key: &[u8]) -> Self {
        CookieSessionBackend {
            key: Rc::new(Key::from_master(key)),
        }
    }
}

impl SessionBackend for CookieSessionBackend {

    type Session = CookieSession;
    type ReadFuture = FutureResult<CookieSession, Error>;

    fn from_request(&self, req: &mut HttpRequest) -> Self::ReadFuture {
        unimplemented!()
    }
}
