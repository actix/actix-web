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
//! [**Identity**](struct.Identity.html) extractor should be used.
//!
//! ```rust
//! use actix_web::*;
//! use actix_identity::{Identity, CookieIdentityPolicy, IdentityService};
//!
//! async fn index(id: Identity) -> String {
//!     // access request identity
//!     if let Some(id) = id.identity() {
//!         format!("Welcome! {}", id)
//!     } else {
//!         "Welcome Anonymous!".to_owned()
//!     }
//! }
//!
//! async fn login(id: Identity) -> HttpResponse {
//!     id.remember("User1".to_owned()); // <- remember identity
//!     HttpResponse::Ok().finish()
//! }
//!
//! async fn logout(id: Identity) -> HttpResponse {
//!     id.forget();                      // <- remove identity
//!     HttpResponse::Ok().finish()
//! }
//!
//! fn main() {
//!     let app = App::new().wrap(IdentityService::new(
//!         // <- create identity middleware
//!         CookieIdentityPolicy::new(&[0; 32])    // <- create cookie identity policy
//!               .name("auth-cookie")
//!               .secure(false)))
//!         .service(web::resource("/index.html").to(index))
//!         .service(web::resource("/login.html").to(login))
//!         .service(web::resource("/logout.html").to(logout));
//! }
//! ```
use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::SystemTime;

use actix_service::{Service, Transform};
use futures::future::{ok, FutureExt, LocalBoxFuture, Ready};
use serde::{Deserialize, Serialize};
use time::Duration;

use actix_web::cookie::{Cookie, CookieJar, Key, SameSite};
use actix_web::dev::{Extensions, Payload, ServiceRequest, ServiceResponse};
use actix_web::error::{Error, Result};
use actix_web::http::header::{self, HeaderValue};
use actix_web::{FromRequest, HttpMessage, HttpRequest};

/// The extractor type to obtain your identity from a request.
///
/// ```rust
/// use actix_web::*;
/// use actix_identity::Identity;
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
        Identity::get_identity(&self.0.extensions())
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

    fn get_identity(extensions: &Extensions) -> Option<String> {
        if let Some(id) = extensions.get::<IdentityItem>() {
            id.id.clone()
        } else {
            None
        }
    }
}

struct IdentityItem {
    id: Option<String>,
    changed: bool,
}

/// Helper trait that allows to get Identity.
///
/// It could be used in middleware but identity policy must be set before any other middleware that needs identity
/// RequestIdentity is implemented both for `ServiceRequest` and `HttpRequest`.
pub trait RequestIdentity {
    fn get_identity(&self) -> Option<String>;
}

impl<T> RequestIdentity for T
where
    T: HttpMessage,
{
    fn get_identity(&self) -> Option<String> {
        Identity::get_identity(&self.extensions())
    }
}

/// Extractor implementation for Identity type.
///
/// ```rust
/// # use actix_web::*;
/// use actix_identity::Identity;
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
impl FromRequest for Identity {
    type Config = ();
    type Error = Error;
    type Future = Ready<Result<Identity, Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ok(Identity(req.clone()))
    }
}

/// Identity policy definition.
pub trait IdentityPolicy: Sized + 'static {
    /// The return type of the middleware
    type Future: Future<Output = Result<Option<String>, Error>>;

    /// The return type of the middleware
    type ResponseFuture: Future<Output = Result<(), Error>>;

    /// Parse the session from request and load data from a service identity.
    fn from_request(&self, request: &mut ServiceRequest) -> Self::Future;

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
/// use actix_identity::{CookieIdentityPolicy, IdentityService};
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

impl<S, T, B> Transform<S> for IdentityService<T>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>
        + 'static,
    S::Future: 'static,
    T: IdentityPolicy,
    B: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = IdentityServiceMiddleware<S, T>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

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

impl<S, T, B> Service for IdentityServiceMiddleware<S, T>
where
    B: 'static,
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>
        + 'static,
    S::Future: 'static,
    T: IdentityPolicy,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        self.service.borrow_mut().poll_ready(cx)
    }

    fn call(&mut self, mut req: ServiceRequest) -> Self::Future {
        let srv = self.service.clone();
        let backend = self.backend.clone();
        let fut = self.backend.from_request(&mut req);

        async move {
            match fut.await {
                Ok(id) => {
                    req.extensions_mut()
                        .insert(IdentityItem { id, changed: false });

                    let mut res = srv.borrow_mut().call(req).await?;
                    let id = res.request().extensions_mut().remove::<IdentityItem>();

                    if let Some(id) = id {
                        match backend.to_response(id.id, id.changed, &mut res).await {
                            Ok(_) => Ok(res),
                            Err(e) => Ok(res.error_response(e)),
                        }
                    } else {
                        Ok(res)
                    }
                }
                Err(err) => Ok(req.error_response(err)),
            }
        }
            .boxed_local()
    }
}

struct CookieIdentityInner {
    key: Key,
    key_v2: Key,
    name: String,
    path: String,
    domain: Option<String>,
    secure: bool,
    max_age: Option<Duration>,
    same_site: Option<SameSite>,
    visit_deadline: Option<Duration>,
    login_deadline: Option<Duration>,
}

#[derive(Deserialize, Serialize, Debug)]
struct CookieValue {
    identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    login_timestamp: Option<SystemTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visit_timestamp: Option<SystemTime>,
}

#[derive(Debug)]
struct CookieIdentityExtention {
    login_timestamp: Option<SystemTime>,
}

impl CookieIdentityInner {
    fn new(key: &[u8]) -> CookieIdentityInner {
        let key_v2: Vec<u8> = key.iter().chain([1, 0, 0, 0].iter()).cloned().collect();
        CookieIdentityInner {
            key: Key::from_master(key),
            key_v2: Key::from_master(&key_v2),
            name: "actix-identity".to_owned(),
            path: "/".to_owned(),
            domain: None,
            secure: true,
            max_age: None,
            same_site: None,
            visit_deadline: None,
            login_deadline: None,
        }
    }

    fn set_cookie<B>(
        &self,
        resp: &mut ServiceResponse<B>,
        value: Option<CookieValue>,
    ) -> Result<()> {
        let add_cookie = value.is_some();
        let val = value.map(|val| {
            if !self.legacy_supported() {
                serde_json::to_string(&val)
            } else {
                Ok(val.identity)
            }
        });
        let mut cookie =
            Cookie::new(self.name.clone(), val.unwrap_or_else(|| Ok(String::new()))?);
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
        let key = if self.legacy_supported() {
            &self.key
        } else {
            &self.key_v2
        };
        if add_cookie {
            jar.private(&key).add(cookie);
        } else {
            jar.add_original(cookie.clone());
            jar.private(&key).remove(cookie);
        }
        for cookie in jar.delta() {
            let val = HeaderValue::from_str(&cookie.to_string())?;
            resp.headers_mut().append(header::SET_COOKIE, val);
        }
        Ok(())
    }

    fn load(&self, req: &ServiceRequest) -> Option<CookieValue> {
        let cookie = req.cookie(&self.name)?;
        let mut jar = CookieJar::new();
        jar.add_original(cookie.clone());
        let res = if self.legacy_supported() {
            jar.private(&self.key).get(&self.name).map(|n| CookieValue {
                identity: n.value().to_string(),
                login_timestamp: None,
                visit_timestamp: None,
            })
        } else {
            None
        };
        res.or_else(|| {
            jar.private(&self.key_v2)
                .get(&self.name)
                .and_then(|c| self.parse(c))
        })
    }

    fn parse(&self, cookie: Cookie) -> Option<CookieValue> {
        let value: CookieValue = serde_json::from_str(cookie.value()).ok()?;
        let now = SystemTime::now();
        if let Some(visit_deadline) = self.visit_deadline {
            if now.duration_since(value.visit_timestamp?).ok()?
                > visit_deadline.to_std().ok()?
            {
                return None;
            }
        }
        if let Some(login_deadline) = self.login_deadline {
            if now.duration_since(value.login_timestamp?).ok()?
                > login_deadline.to_std().ok()?
            {
                return None;
            }
        }
        Some(value)
    }

    fn legacy_supported(&self) -> bool {
        self.visit_deadline.is_none() && self.login_deadline.is_none()
    }

    fn always_update_cookie(&self) -> bool {
        self.visit_deadline.is_some()
    }

    fn requires_oob_data(&self) -> bool {
        self.login_deadline.is_some()
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
/// use actix_web::App;
/// use actix_identity::{CookieIdentityPolicy, IdentityService};
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

    /// Sets the `max-age` field in the session cookie being built with given number of seconds.
    pub fn max_age(self, seconds: i64) -> CookieIdentityPolicy {
        self.max_age_time(Duration::seconds(seconds))
    }

    /// Sets the `max-age` field in the session cookie being built with `chrono::Duration`.
    pub fn max_age_time(mut self, value: Duration) -> CookieIdentityPolicy {
        Rc::get_mut(&mut self.0).unwrap().max_age = Some(value);
        self
    }

    /// Sets the `same_site` field in the session cookie being built.
    pub fn same_site(mut self, same_site: SameSite) -> Self {
        Rc::get_mut(&mut self.0).unwrap().same_site = Some(same_site);
        self
    }

    /// Accepts only users whose cookie has been seen before the given deadline
    ///
    /// By default visit deadline is disabled.
    pub fn visit_deadline(mut self, value: Duration) -> CookieIdentityPolicy {
        Rc::get_mut(&mut self.0).unwrap().visit_deadline = Some(value);
        self
    }

    /// Accepts only users which has been authenticated before the given deadline
    ///
    /// By default login deadline is disabled.
    pub fn login_deadline(mut self, value: Duration) -> CookieIdentityPolicy {
        Rc::get_mut(&mut self.0).unwrap().login_deadline = Some(value);
        self
    }
}

impl IdentityPolicy for CookieIdentityPolicy {
    type Future = Ready<Result<Option<String>, Error>>;
    type ResponseFuture = Ready<Result<(), Error>>;

    fn from_request(&self, req: &mut ServiceRequest) -> Self::Future {
        ok(self.0.load(req).map(
            |CookieValue {
                 identity,
                 login_timestamp,
                 ..
             }| {
                if self.0.requires_oob_data() {
                    req.extensions_mut()
                        .insert(CookieIdentityExtention { login_timestamp });
                }
                identity
            },
        ))
    }

    fn to_response<B>(
        &self,
        id: Option<String>,
        changed: bool,
        res: &mut ServiceResponse<B>,
    ) -> Self::ResponseFuture {
        let _ = if changed {
            let login_timestamp = SystemTime::now();
            self.0.set_cookie(
                res,
                id.map(|identity| CookieValue {
                    identity,
                    login_timestamp: self.0.login_deadline.map(|_| login_timestamp),
                    visit_timestamp: self.0.visit_deadline.map(|_| login_timestamp),
                }),
            )
        } else if self.0.always_update_cookie() && id.is_some() {
            let visit_timestamp = SystemTime::now();
            let login_timestamp = if self.0.requires_oob_data() {
                let CookieIdentityExtention {
                    login_timestamp: lt,
                } = res.request().extensions_mut().remove().unwrap();
                lt
            } else {
                None
            };
            self.0.set_cookie(
                res,
                Some(CookieValue {
                    identity: id.unwrap(),
                    login_timestamp,
                    visit_timestamp: self.0.visit_deadline.map(|_| visit_timestamp),
                }),
            )
        } else {
            Ok(())
        };
        ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Borrow;

    use super::*;
    use actix_web::http::StatusCode;
    use actix_web::test::{self, TestRequest};
    use actix_web::{web, App, Error, HttpResponse};

    const COOKIE_KEY_MASTER: [u8; 32] = [0; 32];
    const COOKIE_NAME: &'static str = "actix_auth";
    const COOKIE_LOGIN: &'static str = "test";

    #[actix_rt::test]
    async fn test_identity() {
        let mut srv = test::init_service(
            App::new()
                .wrap(IdentityService::new(
                    CookieIdentityPolicy::new(&COOKIE_KEY_MASTER)
                        .domain("www.rust-lang.org")
                        .name(COOKIE_NAME)
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
                    id.remember(COOKIE_LOGIN.to_string());
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
        )
        .await;
        let resp =
            test::call_service(&mut srv, TestRequest::with_uri("/index").to_request())
                .await;
        assert_eq!(resp.status(), StatusCode::OK);

        let resp =
            test::call_service(&mut srv, TestRequest::with_uri("/login").to_request())
                .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let c = resp.response().cookies().next().unwrap().to_owned();

        let resp = test::call_service(
            &mut srv,
            TestRequest::with_uri("/index")
                .cookie(c.clone())
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp = test::call_service(
            &mut srv,
            TestRequest::with_uri("/logout")
                .cookie(c.clone())
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key(header::SET_COOKIE))
    }

    #[actix_rt::test]
    async fn test_identity_max_age_time() {
        let duration = Duration::days(1);
        let mut srv = test::init_service(
            App::new()
                .wrap(IdentityService::new(
                    CookieIdentityPolicy::new(&COOKIE_KEY_MASTER)
                        .domain("www.rust-lang.org")
                        .name(COOKIE_NAME)
                        .path("/")
                        .max_age_time(duration)
                        .secure(true),
                ))
                .service(web::resource("/login").to(|id: Identity| {
                    id.remember("test".to_string());
                    HttpResponse::Ok()
                })),
        )
        .await;
        let resp =
            test::call_service(&mut srv, TestRequest::with_uri("/login").to_request())
                .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key(header::SET_COOKIE));
        let c = resp.response().cookies().next().unwrap().to_owned();
        assert_eq!(duration, c.max_age().unwrap());
    }

    #[actix_rt::test]
    async fn test_identity_max_age() {
        let seconds = 60;
        let mut srv = test::init_service(
            App::new()
                .wrap(IdentityService::new(
                    CookieIdentityPolicy::new(&COOKIE_KEY_MASTER)
                        .domain("www.rust-lang.org")
                        .name(COOKIE_NAME)
                        .path("/")
                        .max_age(seconds)
                        .secure(true),
                ))
                .service(web::resource("/login").to(|id: Identity| {
                    id.remember("test".to_string());
                    HttpResponse::Ok()
                })),
        )
        .await;
        let resp =
            test::call_service(&mut srv, TestRequest::with_uri("/login").to_request())
                .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key(header::SET_COOKIE));
        let c = resp.response().cookies().next().unwrap().to_owned();
        assert_eq!(Duration::seconds(seconds as i64), c.max_age().unwrap());
    }

    async fn create_identity_server<
        F: Fn(CookieIdentityPolicy) -> CookieIdentityPolicy + Sync + Send + Clone + 'static,
    >(
        f: F,
    ) -> impl actix_service::Service<
        Request = actix_http::Request,
        Response = ServiceResponse<actix_web::body::Body>,
        Error = Error,
    > {
        test::init_service(
            App::new()
                .wrap(IdentityService::new(f(CookieIdentityPolicy::new(
                    &COOKIE_KEY_MASTER,
                )
                .secure(false)
                .name(COOKIE_NAME))))
                .service(web::resource("/").to(|id: Identity| {
                    async move {
                        let identity = id.identity();
                        if identity.is_none() {
                            id.remember(COOKIE_LOGIN.to_string())
                        }
                        web::Json(identity)
                    }
                })),
        )
        .await
    }

    fn legacy_login_cookie(identity: &'static str) -> Cookie<'static> {
        let mut jar = CookieJar::new();
        jar.private(&Key::from_master(&COOKIE_KEY_MASTER))
            .add(Cookie::new(COOKIE_NAME, identity));
        jar.get(COOKIE_NAME).unwrap().clone()
    }

    fn login_cookie(
        identity: &'static str,
        login_timestamp: Option<SystemTime>,
        visit_timestamp: Option<SystemTime>,
    ) -> Cookie<'static> {
        let mut jar = CookieJar::new();
        let key: Vec<u8> = COOKIE_KEY_MASTER
            .iter()
            .chain([1, 0, 0, 0].iter())
            .map(|e| *e)
            .collect();
        jar.private(&Key::from_master(&key)).add(Cookie::new(
            COOKIE_NAME,
            serde_json::to_string(&CookieValue {
                identity: identity.to_string(),
                login_timestamp,
                visit_timestamp,
            })
            .unwrap(),
        ));
        jar.get(COOKIE_NAME).unwrap().clone()
    }

    async fn assert_logged_in(response: ServiceResponse, identity: Option<&str>) {
        let bytes = test::read_body(response).await;
        let resp: Option<String> = serde_json::from_slice(&bytes[..]).unwrap();
        assert_eq!(resp.as_ref().map(|s| s.borrow()), identity);
    }

    fn assert_legacy_login_cookie(response: &mut ServiceResponse, identity: &str) {
        let mut cookies = CookieJar::new();
        for cookie in response.headers().get_all(header::SET_COOKIE) {
            cookies.add(Cookie::parse(cookie.to_str().unwrap().to_string()).unwrap());
        }
        let cookie = cookies
            .private(&Key::from_master(&COOKIE_KEY_MASTER))
            .get(COOKIE_NAME)
            .unwrap();
        assert_eq!(cookie.value(), identity);
    }

    enum LoginTimestampCheck {
        NoTimestamp,
        NewTimestamp,
        OldTimestamp(SystemTime),
    }

    enum VisitTimeStampCheck {
        NoTimestamp,
        NewTimestamp,
    }

    fn assert_login_cookie(
        response: &mut ServiceResponse,
        identity: &str,
        login_timestamp: LoginTimestampCheck,
        visit_timestamp: VisitTimeStampCheck,
    ) {
        let mut cookies = CookieJar::new();
        for cookie in response.headers().get_all(header::SET_COOKIE) {
            cookies.add(Cookie::parse(cookie.to_str().unwrap().to_string()).unwrap());
        }
        let key: Vec<u8> = COOKIE_KEY_MASTER
            .iter()
            .chain([1, 0, 0, 0].iter())
            .map(|e| *e)
            .collect();
        let cookie = cookies
            .private(&Key::from_master(&key))
            .get(COOKIE_NAME)
            .unwrap();
        let cv: CookieValue = serde_json::from_str(cookie.value()).unwrap();
        assert_eq!(cv.identity, identity);
        let now = SystemTime::now();
        let t30sec_ago = now - Duration::seconds(30).to_std().unwrap();
        match login_timestamp {
            LoginTimestampCheck::NoTimestamp => assert_eq!(cv.login_timestamp, None),
            LoginTimestampCheck::NewTimestamp => assert!(
                t30sec_ago <= cv.login_timestamp.unwrap()
                    && cv.login_timestamp.unwrap() <= now
            ),
            LoginTimestampCheck::OldTimestamp(old_timestamp) => {
                assert_eq!(cv.login_timestamp, Some(old_timestamp))
            }
        }
        match visit_timestamp {
            VisitTimeStampCheck::NoTimestamp => assert_eq!(cv.visit_timestamp, None),
            VisitTimeStampCheck::NewTimestamp => assert!(
                t30sec_ago <= cv.visit_timestamp.unwrap()
                    && cv.visit_timestamp.unwrap() <= now
            ),
        }
    }

    fn assert_no_login_cookie(response: &mut ServiceResponse) {
        let mut cookies = CookieJar::new();
        for cookie in response.headers().get_all(header::SET_COOKIE) {
            cookies.add(Cookie::parse(cookie.to_str().unwrap().to_string()).unwrap());
        }
        assert!(cookies.get(COOKIE_NAME).is_none());
    }

    #[actix_rt::test]
    async fn test_identity_legacy_cookie_is_set() {
        let mut srv = create_identity_server(|c| c).await;
        let mut resp =
            test::call_service(&mut srv, TestRequest::with_uri("/").to_request()).await;
        assert_legacy_login_cookie(&mut resp, COOKIE_LOGIN);
        assert_logged_in(resp, None).await;
    }

    #[actix_rt::test]
    async fn test_identity_legacy_cookie_works() {
        let mut srv = create_identity_server(|c| c).await;
        let cookie = legacy_login_cookie(COOKIE_LOGIN);
        let mut resp = test::call_service(
            &mut srv,
            TestRequest::with_uri("/")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_no_login_cookie(&mut resp);
        assert_logged_in(resp, Some(COOKIE_LOGIN)).await;
    }

    #[actix_rt::test]
    async fn test_identity_legacy_cookie_rejected_if_visit_timestamp_needed() {
        let mut srv =
            create_identity_server(|c| c.visit_deadline(Duration::days(90))).await;
        let cookie = legacy_login_cookie(COOKIE_LOGIN);
        let mut resp = test::call_service(
            &mut srv,
            TestRequest::with_uri("/")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_login_cookie(
            &mut resp,
            COOKIE_LOGIN,
            LoginTimestampCheck::NoTimestamp,
            VisitTimeStampCheck::NewTimestamp,
        );
        assert_logged_in(resp, None).await;
    }

    #[actix_rt::test]
    async fn test_identity_legacy_cookie_rejected_if_login_timestamp_needed() {
        let mut srv =
            create_identity_server(|c| c.login_deadline(Duration::days(90))).await;
        let cookie = legacy_login_cookie(COOKIE_LOGIN);
        let mut resp = test::call_service(
            &mut srv,
            TestRequest::with_uri("/")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_login_cookie(
            &mut resp,
            COOKIE_LOGIN,
            LoginTimestampCheck::NewTimestamp,
            VisitTimeStampCheck::NoTimestamp,
        );
        assert_logged_in(resp, None).await;
    }

    #[actix_rt::test]
    async fn test_identity_cookie_rejected_if_login_timestamp_needed() {
        let mut srv =
            create_identity_server(|c| c.login_deadline(Duration::days(90))).await;
        let cookie = login_cookie(COOKIE_LOGIN, None, Some(SystemTime::now()));
        let mut resp = test::call_service(
            &mut srv,
            TestRequest::with_uri("/")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_login_cookie(
            &mut resp,
            COOKIE_LOGIN,
            LoginTimestampCheck::NewTimestamp,
            VisitTimeStampCheck::NoTimestamp,
        );
        assert_logged_in(resp, None).await;
    }

    #[actix_rt::test]
    async fn test_identity_cookie_rejected_if_visit_timestamp_needed() {
        let mut srv =
            create_identity_server(|c| c.visit_deadline(Duration::days(90))).await;
        let cookie = login_cookie(COOKIE_LOGIN, Some(SystemTime::now()), None);
        let mut resp = test::call_service(
            &mut srv,
            TestRequest::with_uri("/")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_login_cookie(
            &mut resp,
            COOKIE_LOGIN,
            LoginTimestampCheck::NoTimestamp,
            VisitTimeStampCheck::NewTimestamp,
        );
        assert_logged_in(resp, None).await;
    }

    #[actix_rt::test]
    async fn test_identity_cookie_rejected_if_login_timestamp_too_old() {
        let mut srv =
            create_identity_server(|c| c.login_deadline(Duration::days(90))).await;
        let cookie = login_cookie(
            COOKIE_LOGIN,
            Some(SystemTime::now() - Duration::days(180).to_std().unwrap()),
            None,
        );
        let mut resp = test::call_service(
            &mut srv,
            TestRequest::with_uri("/")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_login_cookie(
            &mut resp,
            COOKIE_LOGIN,
            LoginTimestampCheck::NewTimestamp,
            VisitTimeStampCheck::NoTimestamp,
        );
        assert_logged_in(resp, None).await;
    }

    #[actix_rt::test]
    async fn test_identity_cookie_rejected_if_visit_timestamp_too_old() {
        let mut srv =
            create_identity_server(|c| c.visit_deadline(Duration::days(90))).await;
        let cookie = login_cookie(
            COOKIE_LOGIN,
            None,
            Some(SystemTime::now() - Duration::days(180).to_std().unwrap()),
        );
        let mut resp = test::call_service(
            &mut srv,
            TestRequest::with_uri("/")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_login_cookie(
            &mut resp,
            COOKIE_LOGIN,
            LoginTimestampCheck::NoTimestamp,
            VisitTimeStampCheck::NewTimestamp,
        );
        assert_logged_in(resp, None).await;
    }

    #[actix_rt::test]
    async fn test_identity_cookie_not_updated_on_login_deadline() {
        let mut srv =
            create_identity_server(|c| c.login_deadline(Duration::days(90))).await;
        let cookie = login_cookie(COOKIE_LOGIN, Some(SystemTime::now()), None);
        let mut resp = test::call_service(
            &mut srv,
            TestRequest::with_uri("/")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_no_login_cookie(&mut resp);
        assert_logged_in(resp, Some(COOKIE_LOGIN)).await;
    }

    #[actix_rt::test]
    async fn test_identity_cookie_updated_on_visit_deadline() {
        let mut srv = create_identity_server(|c| {
            c.visit_deadline(Duration::days(90))
                .login_deadline(Duration::days(90))
        })
        .await;
        let timestamp = SystemTime::now() - Duration::days(1).to_std().unwrap();
        let cookie = login_cookie(COOKIE_LOGIN, Some(timestamp), Some(timestamp));
        let mut resp = test::call_service(
            &mut srv,
            TestRequest::with_uri("/")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_login_cookie(
            &mut resp,
            COOKIE_LOGIN,
            LoginTimestampCheck::OldTimestamp(timestamp),
            VisitTimeStampCheck::NewTimestamp,
        );
        assert_logged_in(resp, Some(COOKIE_LOGIN)).await;
    }
}
