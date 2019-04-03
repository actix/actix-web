//! Request identity service for Actix applications.
//!
//! [**IdentityService**](struct.IdentityService.html) middleware can be
//! used with different policies types to store identity information.
//!
//! By default, only cookie identity policy is implemented. Other backend
//! implementations can be added separately.
//!
//! [**CookieIdentityPolicy**](struct.CookieIdentityPolicy.html)
//! uses cookies as identity storage.
//!
//! To access current request identity
//! [**Identity**](trait.Identity.html) extractor should be used.
//!
//! ```rust
//! use actix_web::middleware::identity::Identity;
//! use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
//! use actix_web::*;
//!
//! fn index(id: Identity) -> String {
//!     // access request identity
//!     if let Some(id) = id.identity() {
//!         format!("Welcome! {}", id)
//!     } else {
//!         "Welcome Anonymous!".to_owned()
//!     }
//! }
//!
//! fn login(id: Identity) -> HttpResponse {
//!     id.remember("User1".to_owned()); // <- remember identity
//!     HttpResponse::Ok().finish()
//! }
//!
//! fn logout(id: Identity) -> HttpResponse {
//!     id.forget();                      // <- remove identity
//!     HttpResponse::Ok().finish()
//! }
//!
//! fn main() {
//!     let app = App::new().wrap(IdentityService::new(
//!         // <- create identity middleware
//!         CookieIdentityPolicy::new(&[0; 32])    // <- create cookie session backend
//!               .name("auth-cookie")
//!               .secure(false)))
//!         .service(web::resource("/index.html").to(index))
//!         .service(web::resource("/login.html").to(login))
//!         .service(web::resource("/logout.html").to(logout));
//! }
//! ```
use std::cell::RefCell;
use std::rc::Rc;

use actix_service::{Service, Transform};
use futures::future::{ok, Either, FutureResult};
use futures::{Future, IntoFuture, Poll};
use time::Duration;

use crate::cookie::{Cookie, CookieJar, Key, SameSite};
use crate::error::{Error, Result};
use crate::http::header::{self, HeaderValue};
use crate::request::HttpRequest;
use crate::service::{ServiceFromRequest, ServiceRequest, ServiceResponse};
use crate::FromRequest;
use crate::HttpMessage;

/// The extractor type to obtain your identity from a request.
///
/// ```rust
/// use actix_web::*;
/// use actix_web::middleware::identity::Identity;
///
/// fn index(id: Identity) -> Result<String> {
///     // access request identity
///     if let Some(id) = id.identity() {
///         Ok(format!("Welcome! {}", id))
///     } else {
///         Ok("Welcome Anonymous!".to_owned())
///     }
/// }
///
/// fn login(id: Identity) -> HttpResponse {
///     id.remember("User1".to_owned()); // <- remember identity
///     HttpResponse::Ok().finish()
/// }
///
/// fn logout(id: Identity) -> HttpResponse {
///     id.forget(); // <- remove identity
///     HttpResponse::Ok().finish()
/// }
/// # fn main() {}
/// ```
#[derive(Clone)]
pub struct Identity(HttpRequest);

impl Identity {
    /// Return the claimed identity of the user associated request or
    /// ``None`` if no identity can be found associated with the request.
    pub fn identity(&self) -> Option<String> {
        if let Some(id) = self.0.extensions().get::<IdentityItem>() {
            id.id.clone()
        } else {
            None
        }
    }

    /// Remember identity.
    pub fn remember(&self, identity: String) {
        if let Some(id) = self.0.extensions_mut().get_mut::<IdentityItem>() {
            id.id = Some(identity);
            id.changed = true;
        }
    }

    /// This method is used to 'forget' the current identity on subsequent
    /// requests.
    pub fn forget(&self) {
        if let Some(id) = self.0.extensions_mut().get_mut::<IdentityItem>() {
            id.id = None;
            id.changed = true;
        }
    }
}

struct IdentityItem {
    id: Option<String>,
    changed: bool,
}

/// Extractor implementation for Identity type.
///
/// ```rust
/// # use actix_web::*;
/// use actix_web::middleware::identity::Identity;
///
/// fn index(id: Identity) -> String {
///     // access request identity
///     if let Some(id) = id.identity() {
///         format!("Welcome! {}", id)
///     } else {
///         "Welcome Anonymous!".to_owned()
///     }
/// }
/// # fn main() {}
/// ```
impl<P> FromRequest<P> for Identity {
    type Error = Error;
    type Future = Result<Identity, Error>;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        Ok(Identity(req.request().clone()))
    }
}

/// Identity policy definition.
pub trait IdentityPolicy: Sized + 'static {
    /// The return type of the middleware
    type Future: IntoFuture<Item = Option<String>, Error = Error>;

    /// The return type of the middleware
    type ResponseFuture: IntoFuture<Item = (), Error = Error>;

    /// Parse the session from request and load data from a service identity.
    fn from_request<P>(&self, request: &mut ServiceRequest<P>) -> Self::Future;

    /// Write changes to response
    fn to_response<B>(
        &self,
        identity: Option<String>,
        changed: bool,
        response: &mut ServiceResponse<B>,
    ) -> Self::ResponseFuture;
}

/// Request identity middleware
///
/// ```rust
/// use actix_web::App;
/// use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
///
/// fn main() {
///     let app = App::new().wrap(IdentityService::new(
///         // <- create identity middleware
///         CookieIdentityPolicy::new(&[0; 32])    // <- create cookie session backend
///               .name("auth-cookie")
///               .secure(false),
///     ));
/// }
/// ```
pub struct IdentityService<T> {
    backend: Rc<T>,
}

impl<T> IdentityService<T> {
    /// Create new identity service with specified backend.
    pub fn new(backend: T) -> Self {
        IdentityService {
            backend: Rc::new(backend),
        }
    }
}

impl<S, T, P, B> Transform<S> for IdentityService<T>
where
    P: 'static,
    S: Service<Request = ServiceRequest<P>, Response = ServiceResponse<B>> + 'static,
    S::Future: 'static,
    T: IdentityPolicy,
    B: 'static,
{
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse<B>;
    type Error = S::Error;
    type InitError = ();
    type Transform = IdentityServiceMiddleware<S, T>;
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(IdentityServiceMiddleware {
            backend: self.backend.clone(),
            service: Rc::new(RefCell::new(service)),
        })
    }
}

#[doc(hidden)]
pub struct IdentityServiceMiddleware<S, T> {
    backend: Rc<T>,
    service: Rc<RefCell<S>>,
}

impl<S, T, P, B> Service for IdentityServiceMiddleware<S, T>
where
    P: 'static,
    B: 'static,
    S: Service<Request = ServiceRequest<P>, Response = ServiceResponse<B>> + 'static,
    S::Future: 'static,
    T: IdentityPolicy,
{
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse<B>;
    type Error = S::Error;
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.borrow_mut().poll_ready()
    }

    fn call(&mut self, mut req: ServiceRequest<P>) -> Self::Future {
        let srv = self.service.clone();
        let backend = self.backend.clone();

        Box::new(
            self.backend.from_request(&mut req).into_future().then(
                move |res| match res {
                    Ok(id) => {
                        req.extensions_mut()
                            .insert(IdentityItem { id, changed: false });

                        Either::A(srv.borrow_mut().call(req).and_then(move |mut res| {
                            let id =
                                res.request().extensions_mut().remove::<IdentityItem>();

                            if let Some(id) = id {
                                return Either::A(
                                    backend
                                        .to_response(id.id, id.changed, &mut res)
                                        .into_future()
                                        .then(move |t| match t {
                                            Ok(_) => Ok(res),
                                            Err(e) => Ok(res.error_response(e)),
                                        }),
                                );
                            } else {
                                Either::B(ok(res))
                            }
                        }))
                    }
                    Err(err) => Either::B(ok(req.error_response(err))),
                },
            ),
        )
    }
}

struct CookieIdentityInner {
    key: Key,
    name: String,
    path: String,
    domain: Option<String>,
    secure: bool,
    max_age: Option<Duration>,
    same_site: Option<SameSite>,
}

impl CookieIdentityInner {
    fn new(key: &[u8]) -> CookieIdentityInner {
        CookieIdentityInner {
            key: Key::from_master(key),
            name: "actix-identity".to_owned(),
            path: "/".to_owned(),
            domain: None,
            secure: true,
            max_age: None,
            same_site: None,
        }
    }

    fn set_cookie<B>(
        &self,
        resp: &mut ServiceResponse<B>,
        id: Option<String>,
    ) -> Result<()> {
        let some = id.is_some();
        {
            let id = id.unwrap_or_else(String::new);
            let mut cookie = Cookie::new(self.name.clone(), id);
            cookie.set_path(self.path.clone());
            cookie.set_secure(self.secure);
            cookie.set_http_only(true);

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
            if some {
                jar.private(&self.key).add(cookie);
            } else {
                jar.add_original(cookie.clone());
                jar.private(&self.key).remove(cookie);
            }

            for cookie in jar.delta() {
                let val = HeaderValue::from_str(&cookie.to_string())?;
                resp.headers_mut().append(header::SET_COOKIE, val);
            }
        }

        Ok(())
    }

    fn load<T>(&self, req: &ServiceRequest<T>) -> Option<String> {
        if let Ok(cookies) = req.cookies() {
            for cookie in cookies.iter() {
                if cookie.name() == self.name {
                    let mut jar = CookieJar::new();
                    jar.add_original(cookie.clone());

                    let cookie_opt = jar.private(&self.key).get(&self.name);
                    if let Some(cookie) = cookie_opt {
                        return Some(cookie.value().into());
                    }
                }
            }
        }
        None
    }
}

/// Use cookies for request identity storage.
///
/// The constructors take a key as an argument.
/// This is the private key for cookie - when this value is changed,
/// all identities are lost. The constructors will panic if the key is less
/// than 32 bytes in length.
///
/// # Example
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
/// use actix_web::App;
///
/// fn main() {
///     let app = App::new().wrap(IdentityService::new(
///         // <- create identity middleware
///         CookieIdentityPolicy::new(&[0; 32])  // <- construct cookie policy
///                .domain("www.rust-lang.org")
///                .name("actix_auth")
///                .path("/")
///                .secure(true),
///     ));
/// }
/// ```
pub struct CookieIdentityPolicy(Rc<CookieIdentityInner>);

impl CookieIdentityPolicy {
    /// Construct new `CookieIdentityPolicy` instance.
    ///
    /// Panics if key length is less than 32 bytes.
    pub fn new(key: &[u8]) -> CookieIdentityPolicy {
        CookieIdentityPolicy(Rc::new(CookieIdentityInner::new(key)))
    }

    /// Sets the `path` field in the session cookie being built.
    pub fn path<S: Into<String>>(mut self, value: S) -> CookieIdentityPolicy {
        Rc::get_mut(&mut self.0).unwrap().path = value.into();
        self
    }

    /// Sets the `name` field in the session cookie being built.
    pub fn name<S: Into<String>>(mut self, value: S) -> CookieIdentityPolicy {
        Rc::get_mut(&mut self.0).unwrap().name = value.into();
        self
    }

    /// Sets the `domain` field in the session cookie being built.
    pub fn domain<S: Into<String>>(mut self, value: S) -> CookieIdentityPolicy {
        Rc::get_mut(&mut self.0).unwrap().domain = Some(value.into());
        self
    }

    /// Sets the `secure` field in the session cookie being built.
    ///
    /// If the `secure` field is set, a cookie will only be transmitted when the
    /// connection is secure - i.e. `https`
    pub fn secure(mut self, value: bool) -> CookieIdentityPolicy {
        Rc::get_mut(&mut self.0).unwrap().secure = value;
        self
    }

    /// Sets the `max-age` field in the session cookie being built.
    pub fn max_age(mut self, value: Duration) -> CookieIdentityPolicy {
        Rc::get_mut(&mut self.0).unwrap().max_age = Some(value);
        self
    }

    /// Sets the `same_site` field in the session cookie being built.
    pub fn same_site(mut self, same_site: SameSite) -> Self {
        Rc::get_mut(&mut self.0).unwrap().same_site = Some(same_site);
        self
    }
}

impl IdentityPolicy for CookieIdentityPolicy {
    type Future = Result<Option<String>, Error>;
    type ResponseFuture = Result<(), Error>;

    fn from_request<P>(&self, req: &mut ServiceRequest<P>) -> Self::Future {
        Ok(self.0.load(req))
    }

    fn to_response<B>(
        &self,
        id: Option<String>,
        changed: bool,
        res: &mut ServiceResponse<B>,
    ) -> Self::ResponseFuture {
        if changed {
            let _ = self.0.set_cookie(res, id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::StatusCode;
    use crate::test::{self, TestRequest};
    use crate::{web, App, HttpResponse};

    #[test]
    fn test_identity() {
        let mut srv = test::init_service(
            App::new()
                .wrap(IdentityService::new(
                    CookieIdentityPolicy::new(&[0; 32])
                        .domain("www.rust-lang.org")
                        .name("actix_auth")
                        .path("/")
                        .secure(true),
                ))
                .service(web::resource("/index").to(|id: Identity| {
                    if id.identity().is_some() {
                        HttpResponse::Created()
                    } else {
                        HttpResponse::Ok()
                    }
                }))
                .service(web::resource("/login").to(|id: Identity| {
                    id.remember("test".to_string());
                    HttpResponse::Ok()
                }))
                .service(web::resource("/logout").to(|id: Identity| {
                    if id.identity().is_some() {
                        id.forget();
                        HttpResponse::Ok()
                    } else {
                        HttpResponse::BadRequest()
                    }
                })),
        );
        let resp =
            test::call_success(&mut srv, TestRequest::with_uri("/index").to_request());
        assert_eq!(resp.status(), StatusCode::OK);

        let resp =
            test::call_success(&mut srv, TestRequest::with_uri("/login").to_request());
        assert_eq!(resp.status(), StatusCode::OK);
        let c = resp.response().cookies().next().unwrap().to_owned();

        let resp = test::call_success(
            &mut srv,
            TestRequest::with_uri("/index")
                .cookie(c.clone())
                .to_request(),
        );
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp = test::call_success(
            &mut srv,
            TestRequest::with_uri("/logout")
                .cookie(c.clone())
                .to_request(),
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key(header::SET_COOKIE))
    }
}
