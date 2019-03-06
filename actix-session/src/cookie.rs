//! Cookie session.
//!
//! [**CookieSession**](struct.CookieSession.html)
//! uses cookies as session storage. `CookieSession` creates sessions
//! which are limited to storing fewer than 4000 bytes of data, as the payload
//! must fit into a single cookie. An internal server error is generated if a
//! session contains more than 4000 bytes.
//!
//! A cookie may have a security policy of *signed* or *private*. Each has
//! a respective `CookieSession` constructor.
//!
//! A *signed* cookie may be viewed but not modified by the client. A *private*
//! cookie may neither be viewed nor modified by the client.
//!
//! The constructors take a key as an argument. This is the private key
//! for cookie session - when this value is changed, all session data is lost.

use std::collections::HashMap;
use std::rc::Rc;

use actix_service::{Service, Transform};
use actix_web::http::{header::SET_COOKIE, HeaderValue};
use actix_web::{Error, HttpMessage, ResponseError, ServiceRequest, ServiceResponse};
use cookie::{Cookie, CookieJar, Key, SameSite};
use derive_more::{Display, From};
use futures::future::{ok, Future, FutureResult};
use futures::Poll;
use serde_json::error::Error as JsonError;
use time::Duration;

use crate::Session;

/// Errors that can occur during handling cookie session
#[derive(Debug, From, Display)]
pub enum CookieSessionError {
    /// Size of the serialized session is greater than 4000 bytes.
    #[display(fmt = "Size of the serialized session is greater than 4000 bytes.")]
    Overflow,
    /// Fail to serialize session.
    #[display(fmt = "Fail to serialize session")]
    Serialize(JsonError),
}

impl ResponseError for CookieSessionError {}

enum CookieSecurity {
    Signed,
    Private,
}

struct CookieSessionInner {
    key: Key,
    security: CookieSecurity,
    name: String,
    path: String,
    domain: Option<String>,
    secure: bool,
    http_only: bool,
    max_age: Option<Duration>,
    same_site: Option<SameSite>,
}

impl CookieSessionInner {
    fn new(key: &[u8], security: CookieSecurity) -> CookieSessionInner {
        CookieSessionInner {
            security,
            key: Key::from_master(key),
            name: "actix-session".to_owned(),
            path: "/".to_owned(),
            domain: None,
            secure: true,
            http_only: true,
            max_age: None,
            same_site: None,
        }
    }

    fn set_cookie<B>(
        &self,
        res: &mut ServiceResponse<B>,
        state: impl Iterator<Item = (String, String)>,
    ) -> Result<(), Error> {
        let state: HashMap<String, String> = state.collect();
        let value =
            serde_json::to_string(&state).map_err(CookieSessionError::Serialize)?;
        if value.len() > 4064 {
            return Err(CookieSessionError::Overflow.into());
        }

        let mut cookie = Cookie::new(self.name.clone(), value);
        cookie.set_path(self.path.clone());
        cookie.set_secure(self.secure);
        cookie.set_http_only(self.http_only);

        if let Some(ref domain) = self.domain {
            cookie.set_domain(domain.clone());
        }

        if let Some(max_age) = self.max_age {
            cookie.set_max_age(max_age);
        }

        if let Some(same_site) = self.same_site {
            cookie.set_same_site(same_site);
        }

        let mut jar = CookieJar::new();

        match self.security {
            CookieSecurity::Signed => jar.signed(&self.key).add(cookie),
            CookieSecurity::Private => jar.private(&self.key).add(cookie),
        }

        for cookie in jar.delta() {
            let val = HeaderValue::from_str(&cookie.encoded().to_string())?;
            res.headers_mut().append(SET_COOKIE, val);
        }

        Ok(())
    }

    fn load<P>(&self, req: &ServiceRequest<P>) -> HashMap<String, String> {
        if let Ok(cookies) = req.cookies() {
            for cookie in cookies.iter() {
                if cookie.name() == self.name {
                    let mut jar = CookieJar::new();
                    jar.add_original(cookie.clone());

                    let cookie_opt = match self.security {
                        CookieSecurity::Signed => jar.signed(&self.key).get(&self.name),
                        CookieSecurity::Private => {
                            jar.private(&self.key).get(&self.name)
                        }
                    };
                    if let Some(cookie) = cookie_opt {
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

/// Use cookies for session storage.
///
/// `CookieSession` creates sessions which are limited to storing
/// fewer than 4000 bytes of data (as the payload must fit into a single
/// cookie). An Internal Server Error is generated if the session contains more
/// than 4000 bytes.
///
/// A cookie may have a security policy of *signed* or *private*. Each has a
/// respective `CookieSessionBackend` constructor.
///
/// A *signed* cookie is stored on the client as plaintext alongside
/// a signature such that the cookie may be viewed but not modified by the
/// client.
///
/// A *private* cookie is stored on the client as encrypted text
/// such that it may neither be viewed nor modified by the client.
///
/// The constructors take a key as an argument.
/// This is the private key for cookie session - when this value is changed,
/// all session data is lost. The constructors will panic if the key is less
/// than 32 bytes in length.
///
/// The backend relies on `cookie` crate to create and read cookies.
/// By default all cookies are percent encoded, but certain symbols may
/// cause troubles when reading cookie, if they are not properly percent encoded.
///
/// # Example
///
/// ```rust
/// use actix_session::CookieSession;
/// use actix_web::{App, HttpResponse, HttpServer};
///
/// fn main() {
///     let app = App::new().middleware(
///         CookieSession::signed(&[0; 32])
///             .domain("www.rust-lang.org")
///             .name("actix_session")
///             .path("/")
///             .secure(true))
///         .resource("/", |r| r.to(|| HttpResponse::Ok()));
/// }
/// ```
pub struct CookieSession(Rc<CookieSessionInner>);

impl CookieSession {
    /// Construct new *signed* `CookieSessionBackend` instance.
    ///
    /// Panics if key length is less than 32 bytes.
    pub fn signed(key: &[u8]) -> CookieSession {
        CookieSession(Rc::new(CookieSessionInner::new(
            key,
            CookieSecurity::Signed,
        )))
    }

    /// Construct new *private* `CookieSessionBackend` instance.
    ///
    /// Panics if key length is less than 32 bytes.
    pub fn private(key: &[u8]) -> CookieSession {
        CookieSession(Rc::new(CookieSessionInner::new(
            key,
            CookieSecurity::Private,
        )))
    }

    /// Sets the `path` field in the session cookie being built.
    pub fn path<S: Into<String>>(mut self, value: S) -> CookieSession {
        Rc::get_mut(&mut self.0).unwrap().path = value.into();
        self
    }

    /// Sets the `name` field in the session cookie being built.
    pub fn name<S: Into<String>>(mut self, value: S) -> CookieSession {
        Rc::get_mut(&mut self.0).unwrap().name = value.into();
        self
    }

    /// Sets the `domain` field in the session cookie being built.
    pub fn domain<S: Into<String>>(mut self, value: S) -> CookieSession {
        Rc::get_mut(&mut self.0).unwrap().domain = Some(value.into());
        self
    }

    /// Sets the `secure` field in the session cookie being built.
    ///
    /// If the `secure` field is set, a cookie will only be transmitted when the
    /// connection is secure - i.e. `https`
    pub fn secure(mut self, value: bool) -> CookieSession {
        Rc::get_mut(&mut self.0).unwrap().secure = value;
        self
    }

    /// Sets the `http_only` field in the session cookie being built.
    pub fn http_only(mut self, value: bool) -> CookieSession {
        Rc::get_mut(&mut self.0).unwrap().http_only = value;
        self
    }

    /// Sets the `same_site` field in the session cookie being built.
    pub fn same_site(mut self, value: SameSite) -> CookieSession {
        Rc::get_mut(&mut self.0).unwrap().same_site = Some(value);
        self
    }

    /// Sets the `max-age` field in the session cookie being built.
    pub fn max_age(mut self, value: Duration) -> CookieSession {
        Rc::get_mut(&mut self.0).unwrap().max_age = Some(value);
        self
    }
}

impl<S, P, B: 'static> Transform<S, ServiceRequest<P>> for CookieSession
where
    S: Service<ServiceRequest<P>, Response = ServiceResponse<B>>,
    S::Future: 'static,
    S::Error: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = S::Error;
    type InitError = ();
    type Transform = CookieSessionMiddleware<S>;
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(CookieSessionMiddleware {
            service,
            inner: self.0.clone(),
        })
    }
}

/// Cookie session middleware
pub struct CookieSessionMiddleware<S> {
    service: S,
    inner: Rc<CookieSessionInner>,
}

impl<S, P, B: 'static> Service<ServiceRequest<P>> for CookieSessionMiddleware<S>
where
    S: Service<ServiceRequest<P>, Response = ServiceResponse<B>>,
    S::Future: 'static,
    S::Error: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = S::Error;
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        //self.service.poll_ready().map_err(|e| e.into())
        self.service.poll_ready()
    }

    fn call(&mut self, mut req: ServiceRequest<P>) -> Self::Future {
        let inner = self.inner.clone();
        let state = self.inner.load(&req);
        Session::set_session(state.into_iter(), &mut req);

        Box::new(self.service.call(req).map(move |mut res| {
            if let Some(state) = Session::get_changes(&mut res) {
                res.checked_expr(|res| inner.set_cookie(res, state))
            } else {
                res
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, App};

    #[test]
    fn cookie_session() {
        let mut app = test::init_service(
            App::new()
                .middleware(CookieSession::signed(&[0; 32]).secure(false))
                .resource("/", |r| {
                    r.to(|ses: Session| {
                        let _ = ses.set("counter", 100);
                        "test"
                    })
                }),
        );

        let request = test::TestRequest::get().to_request();
        let response = test::block_on(app.call(request)).unwrap();
        assert!(response
            .cookies()
            .find(|c| c.name() == "actix-session")
            .is_some());
    }

    #[test]
    fn cookie_session_extractor() {
        let mut app = test::init_service(
            App::new()
                .middleware(CookieSession::signed(&[0; 32]).secure(false))
                .resource("/", |r| {
                    r.to(|ses: Session| {
                        let _ = ses.set("counter", 100);
                        "test"
                    })
                }),
        );

        let request = test::TestRequest::get().to_request();
        let response = test::block_on(app.call(request)).unwrap();
        assert!(response
            .cookies()
            .find(|c| c.name() == "actix-session")
            .is_some());
    }
}
