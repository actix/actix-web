//! User sessions.
//!
//! Actix provides a general solution for session management. Session
//! middlewares could provide different implementations which could
//! be accessed via general session api.
//!
//! By default, only cookie session backend is implemented. Other
//! backend implementations can be added.
//!
//! In general, you insert a *session* middleware and initialize it
//! , such as a `CookieSessionBackend`. To access session data,
//! [*Session*](struct.Session.html) extractor must be used. Session
//! extractor allows us to get or set session data.
//!
//! ```rust
//! use actix_web::{web, App, HttpServer, HttpResponse, Error};
//! use actix_session::{Session, CookieSession};
//!
//! fn index(session: Session) -> Result<&'static str, Error> {
//!     // access session data
//!     if let Some(count) = session.get::<i32>("counter")? {
//!         println!("SESSION value: {}", count);
//!         session.set("counter", count+1)?;
//!     } else {
//!         session.set("counter", 1)?;
//!     }
//!
//!     Ok("Welcome!")
//! }
//!
//! fn main() -> std::io::Result<()> {
//! # std::thread::spawn(||
//!     HttpServer::new(
//!         || App::new().wrap(
//!               CookieSession::signed(&[0; 32]) // <- create cookie based session middleware
//!                     .secure(false)
//!              )
//!             .service(web::resource("/").to(|| HttpResponse::Ok())))
//!         .bind("127.0.0.1:59880")?
//!         .run()
//! # );
//! # Ok(())
//! }
//! ```
use std::cell::RefCell;
use std::rc::Rc;

use actix_web::dev::{Extensions, Payload, ServiceRequest, ServiceResponse};
use actix_web::{Error, FromRequest, HttpMessage, HttpRequest};
use hashbrown::HashMap;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json;

#[cfg(feature = "cookie-session")]
mod cookie;
#[cfg(feature = "cookie-session")]
pub use crate::cookie::CookieSession;

/// The high-level interface you use to modify session data.
///
/// Session object could be obtained with
/// [`RequestSession::session`](trait.RequestSession.html#tymethod.session)
/// method. `RequestSession` trait is implemented for `HttpRequest`.
///
/// ```rust
/// use actix_session::Session;
/// use actix_web::*;
///
/// fn index(session: Session) -> Result<&'static str> {
///     // access session data
///     if let Some(count) = session.get::<i32>("counter")? {
///         session.set("counter", count + 1)?;
///     } else {
///         session.set("counter", 1)?;
///     }
///
///     Ok("Welcome!")
/// }
/// # fn main() {}
/// ```
pub struct Session(Rc<RefCell<SessionInner>>);

/// Helper trait that allows to get session
pub trait UserSession {
    fn get_session(&mut self) -> Session;
}

impl UserSession for HttpRequest {
    fn get_session(&mut self) -> Session {
        Session::get_session(&mut *self.extensions_mut())
    }
}

impl UserSession for ServiceRequest {
    fn get_session(&mut self) -> Session {
        Session::get_session(&mut *self.extensions_mut())
    }
}

#[derive(Default)]
struct SessionInner {
    state: HashMap<String, String>,
    changed: bool,
}

impl Session {
    /// Get a `value` from the session.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, Error> {
        if let Some(s) = self.0.borrow().state.get(key) {
            Ok(Some(serde_json::from_str(s)?))
        } else {
            Ok(None)
        }
    }

    /// Set a `value` from the session.
    pub fn set<T: Serialize>(&self, key: &str, value: T) -> Result<(), Error> {
        let mut inner = self.0.borrow_mut();
        inner.changed = true;
        inner
            .state
            .insert(key.to_owned(), serde_json::to_string(&value)?);
        Ok(())
    }

    /// Remove value from the session.
    pub fn remove(&self, key: &str) {
        let mut inner = self.0.borrow_mut();
        inner.changed = true;
        inner.state.remove(key);
    }

    /// Clear the session.
    pub fn clear(&self) {
        let mut inner = self.0.borrow_mut();
        inner.changed = true;
        inner.state.clear()
    }

    pub fn set_session(
        data: impl Iterator<Item = (String, String)>,
        req: &mut ServiceRequest,
    ) {
        let session = Session::get_session(&mut *req.extensions_mut());
        let mut inner = session.0.borrow_mut();
        inner.state.extend(data);
    }

    pub fn get_changes<B>(
        res: &mut ServiceResponse<B>,
    ) -> Option<impl Iterator<Item = (String, String)>> {
        if let Some(s_impl) = res
            .request()
            .extensions()
            .get::<Rc<RefCell<SessionInner>>>()
        {
            let state =
                std::mem::replace(&mut s_impl.borrow_mut().state, HashMap::new());
            Some(state.into_iter())
        } else {
            None
        }
    }

    fn get_session(extensions: &mut Extensions) -> Session {
        if let Some(s_impl) = extensions.get::<Rc<RefCell<SessionInner>>>() {
            return Session(Rc::clone(&s_impl));
        }
        let inner = Rc::new(RefCell::new(SessionInner::default()));
        extensions.insert(inner.clone());
        Session(inner)
    }
}

/// Extractor implementation for Session type.
///
/// ```rust
/// # use actix_web::*;
/// use actix_session::Session;
///
/// fn index(session: Session) -> Result<&'static str> {
///     // access session data
///     if let Some(count) = session.get::<i32>("counter")? {
///         session.set("counter", count + 1)?;
///     } else {
///         session.set("counter", 1)?;
///     }
///
///     Ok("Welcome!")
/// }
/// # fn main() {}
/// ```
impl FromRequest for Session {
    type Error = Error;
    type Future = Result<Session, Error>;
    type Config = ();

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        Ok(Session::get_session(&mut *req.extensions_mut()))
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{test, HttpResponse};

    use super::*;

    #[test]
    fn session() {
        let mut req = test::TestRequest::default().to_srv_request();

        Session::set_session(
            vec![("key".to_string(), "\"value\"".to_string())].into_iter(),
            &mut req,
        );
        let session = Session::get_session(&mut *req.extensions_mut());
        let res = session.get::<String>("key").unwrap();
        assert_eq!(res, Some("value".to_string()));

        session.set("key2", "value2".to_string()).unwrap();
        session.remove("key");

        let mut res = req.into_response(HttpResponse::Ok().finish());
        let changes: Vec<_> = Session::get_changes(&mut res).unwrap().collect();
        assert_eq!(changes, [("key2".to_string(), "\"value2\"".to_string())]);
    }

    #[test]
    fn get_session() {
        let mut req = test::TestRequest::default().to_srv_request();

        Session::set_session(
            vec![("key".to_string(), "\"value\"".to_string())].into_iter(),
            &mut req,
        );

        let session = req.get_session();
        let res = session.get::<String>("key").unwrap();
        assert_eq!(res, Some("value".to_string()));
    }
}
