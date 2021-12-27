//! Route guards.
//!
//! Guards are one of the ways how actix-web router chooses a handler service. In essence it is just
//! a function that accepts a reference to a `RequestHead` instance and returns a boolean. It is
//! possible to add guards to *scopes*, *resources* and *routes*. Actix provide several guards by
//! default, like various HTTP methods, header, etc. To become a guard, type must implement the
//! `Guard` trait. Simple functions could be guards as well.
//!
//! Guards can not modify the request object. But it is possible to store extra attributes on a
//! request by using the `Extensions` container. Extensions containers are available via the
//! `RequestHead::extensions()` method.
//!
//! ```
//! use actix_web::{web, http, dev, guard, App, HttpResponse};
//!
//! App::new().service(web::resource("/index.html").route(
//!     web::route()
//!          .guard(guard::Post())
//!          .guard(guard::fn_guard(|ctx| ctx.head().method == http::Method::GET))
//!          .to(|| HttpResponse::MethodNotAllowed()))
//! );
//! ```

use std::{
    cell::{Ref, RefMut},
    convert::TryFrom,
    rc::Rc,
};

use actix_http::{header, uri::Uri, Extensions, Method as HttpMethod, RequestHead};

use crate::service::ServiceRequest;

#[derive(Debug)]
pub struct GuardContext<'a> {
    pub(crate) req: &'a ServiceRequest,
}

impl<'a> GuardContext<'a> {
    #[inline]
    pub fn head(&self) -> &RequestHead {
        self.req.head()
    }

    #[inline]
    pub fn req_data(&self) -> Ref<'a, Extensions> {
        self.req.req_data()
    }

    #[inline]
    pub fn req_data_mut(&self) -> RefMut<'a, Extensions> {
        self.req.req_data_mut()
    }
}

/// Trait defines resource guards. Guards are used for route selection.
///
/// Guards can not modify the request object. But it is possible to store extra attributes on a
/// request by using the `Extensions` container. Extensions containers are available via the
/// `RequestHead::extensions()` method.
pub trait Guard {
    /// Check if request matches predicate
    fn check(&self, ctx: &GuardContext<'_>) -> bool;
}

impl Guard for Rc<dyn Guard> {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        (**self).check(ctx)
    }
}

/// Create guard object for supplied function.
///
/// ```
/// use actix_web::{guard, web, App, HttpResponse};
///
/// App::new().service(
///         web::resource("/index.html").route(
///             web::route()
///                 .guard(guard::fn_guard(|ctx| {
///                     ctx.head().headers().contains_key("content-type")
///                 }))
///                 .to(|| HttpResponse::MethodNotAllowed()),
///         ),
///     );
/// ```
pub fn fn_guard<F>(f: F) -> impl Guard
where
    F: Fn(&GuardContext<'_>) -> bool,
{
    FnGuard(f)
}

struct FnGuard<F: Fn(&GuardContext<'_>) -> bool>(F);

impl<F> Guard for FnGuard<F>
where
    F: Fn(&GuardContext<'_>) -> bool,
{
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        (self.0)(ctx)
    }
}

impl<F> Guard for F
where
    F: Fn(&GuardContext<'_>) -> bool,
{
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        (self)(ctx)
    }
}

/// Return guard that matches if any of supplied guards.
///
/// ```
/// use actix_web::{web, guard, App, HttpResponse};
///
/// App::new().service(web::resource("/index.html").route(
///     web::route()
///          .guard(guard::Any(guard::Get()).or(guard::Post()))
///          .to(|| HttpResponse::MethodNotAllowed()))
/// );
/// ```
#[allow(non_snake_case)]
pub fn Any<F: Guard + 'static>(guard: F) -> AnyGuard {
    AnyGuard {
        guards: vec![Box::new(guard)],
    }
}

/// Matches any of supplied guards.
pub struct AnyGuard {
    guards: Vec<Box<dyn Guard>>,
}

impl AnyGuard {
    /// Add guard to a list of guards to check
    pub fn or<F: Guard + 'static>(mut self, guard: F) -> Self {
        self.guards.push(Box::new(guard));
        self
    }
}

impl Guard for AnyGuard {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        for guard in &self.guards {
            if guard.check(ctx) {
                return true;
            }
        }

        false
    }
}

/// Return guard that matches if all of the supplied guards.
///
/// ```
/// use actix_web::{guard, web, App, HttpResponse};
///
/// App::new().service(web::resource("/index.html").route(
///     web::route()
///         .guard(
///             guard::All(guard::Get()).and(guard::Header("content-type", "text/plain")))
///         .to(|| HttpResponse::MethodNotAllowed()))
/// );
/// ```
#[allow(non_snake_case)]
pub fn All<F: Guard + 'static>(guard: F) -> AllGuard {
    AllGuard {
        guards: vec![Box::new(guard)],
    }
}

/// Matches if all of supplied guards.
pub struct AllGuard {
    guards: Vec<Box<dyn Guard>>,
}

impl AllGuard {
    /// Add new guard to the list of guards to check
    pub fn and<F: Guard + 'static>(mut self, guard: F) -> Self {
        self.guards.push(Box::new(guard));
        self
    }
}

impl Guard for AllGuard {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        for guard in &self.guards {
            if !guard.check(ctx) {
                return false;
            }
        }
        true
    }
}

/// Return guard that matches if supplied guard does not match.
#[allow(non_snake_case)]
pub fn Not<F: Guard + 'static>(guard: F) -> impl Guard {
    NotGuard(Box::new(guard))
}

struct NotGuard(Box<dyn Guard>);

impl Guard for NotGuard {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        !self.0.check(ctx)
    }
}

/// HTTP method guard.
struct MethodGuard(HttpMethod);

impl Guard for MethodGuard {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        ctx.head().method == self.0
    }
}

/// Guard to match *GET* HTTP method.
#[allow(non_snake_case)]
pub fn Get() -> impl Guard {
    MethodGuard(HttpMethod::GET)
}

/// Predicate to match *POST* HTTP method.
#[allow(non_snake_case)]
pub fn Post() -> impl Guard {
    MethodGuard(HttpMethod::POST)
}

/// Predicate to match *PUT* HTTP method.
#[allow(non_snake_case)]
pub fn Put() -> impl Guard {
    MethodGuard(HttpMethod::PUT)
}

/// Predicate to match *DELETE* HTTP method.
#[allow(non_snake_case)]
pub fn Delete() -> impl Guard {
    MethodGuard(HttpMethod::DELETE)
}

/// Predicate to match *HEAD* HTTP method.
#[allow(non_snake_case)]
pub fn Head() -> impl Guard {
    MethodGuard(HttpMethod::HEAD)
}

/// Predicate to match *OPTIONS* HTTP method.
#[allow(non_snake_case)]
pub fn Options() -> impl Guard {
    MethodGuard(HttpMethod::OPTIONS)
}

/// Predicate to match *CONNECT* HTTP method.
#[allow(non_snake_case)]
pub fn Connect() -> impl Guard {
    MethodGuard(HttpMethod::CONNECT)
}

/// Predicate to match *PATCH* HTTP method.
#[allow(non_snake_case)]
pub fn Patch() -> impl Guard {
    MethodGuard(HttpMethod::PATCH)
}

/// Predicate to match *TRACE* HTTP method.
#[allow(non_snake_case)]
pub fn Trace() -> impl Guard {
    MethodGuard(HttpMethod::TRACE)
}

/// Predicate to match specified HTTP method.
#[allow(non_snake_case)]
pub fn Method(method: HttpMethod) -> impl Guard {
    MethodGuard(method)
}

/// Return predicate that matches if request contains specified header and value.
#[allow(non_snake_case)]
pub fn Header(name: &'static str, value: &'static str) -> impl Guard {
    HeaderGuard(
        header::HeaderName::try_from(name).unwrap(),
        header::HeaderValue::from_static(value),
    )
}

#[doc(hidden)]
struct HeaderGuard(header::HeaderName, header::HeaderValue);

impl Guard for HeaderGuard {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        if let Some(val) = ctx.head().headers.get(&self.0) {
            return val == self.1;
        }

        false
    }
}

/// Return predicate that matches if request contains specified Host name.
///
/// ```
/// use actix_web::{web, guard::Host, App, HttpResponse};
///
/// App::new().service(
///     web::resource("/index.html")
///         .guard(Host("www.rust-lang.org"))
///         .to(|| HttpResponse::MethodNotAllowed())
/// );
/// ```
#[allow(non_snake_case)]
pub fn Host<H: AsRef<str>>(host: H) -> HostGuard {
    HostGuard {
        host: host.as_ref().to_string(),
        scheme: None,
    }
}

fn get_host_uri(req: &RequestHead) -> Option<Uri> {
    req.headers
        .get(header::HOST)
        .and_then(|host_value| host_value.to_str().ok())
        .or_else(|| req.uri.host())
        .map(|host| host.parse().ok())
        .and_then(|host_success| host_success)
}

#[doc(hidden)]
pub struct HostGuard {
    host: String,
    scheme: Option<String>,
}

impl HostGuard {
    /// Set request scheme to match
    pub fn scheme<H: AsRef<str>>(mut self, scheme: H) -> HostGuard {
        self.scheme = Some(scheme.as_ref().to_string());
        self
    }
}

impl Guard for HostGuard {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        let req_host_uri = if let Some(uri) = get_host_uri(ctx.head()) {
            uri
        } else {
            return false;
        };

        if let Some(uri_host) = req_host_uri.host() {
            if self.host != uri_host {
                return false;
            }
        } else {
            return false;
        }

        if let Some(ref scheme) = self.scheme {
            if let Some(ref req_host_uri_scheme) = req_host_uri.scheme_str() {
                return scheme == req_host_uri_scheme;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use actix_http::{header, Method};

    use super::*;
    use crate::test::TestRequest;

    #[test]
    fn test_header() {
        let req = TestRequest::default()
            .insert_header((header::TRANSFER_ENCODING, "chunked"))
            .to_srv_request();

        let pred = Header("transfer-encoding", "chunked");
        assert!(pred.check(&req.guard_ctx()));

        let pred = Header("transfer-encoding", "other");
        assert!(!pred.check(&req.guard_ctx()));

        let pred = Header("content-type", "other");
        assert!(!pred.check(&req.guard_ctx()));
    }

    #[test]
    fn test_host() {
        let req = TestRequest::default()
            .insert_header((
                header::HOST,
                header::HeaderValue::from_static("www.rust-lang.org"),
            ))
            .to_srv_request();

        let pred = Host("www.rust-lang.org");
        assert!(pred.check(&req.guard_ctx()));

        let pred = Host("www.rust-lang.org").scheme("https");
        assert!(pred.check(&req.guard_ctx()));

        let pred = Host("blog.rust-lang.org");
        assert!(!pred.check(&req.guard_ctx()));

        let pred = Host("blog.rust-lang.org").scheme("https");
        assert!(!pred.check(&req.guard_ctx()));

        let pred = Host("crates.io");
        assert!(!pred.check(&req.guard_ctx()));

        let pred = Host("localhost");
        assert!(!pred.check(&req.guard_ctx()));
    }

    #[test]
    fn test_host_scheme() {
        let req = TestRequest::default()
            .insert_header((
                header::HOST,
                header::HeaderValue::from_static("https://www.rust-lang.org"),
            ))
            .to_srv_request();

        let pred = Host("www.rust-lang.org").scheme("https");
        assert!(pred.check(&req.guard_ctx()));

        let pred = Host("www.rust-lang.org");
        assert!(pred.check(&req.guard_ctx()));

        let pred = Host("www.rust-lang.org").scheme("http");
        assert!(!pred.check(&req.guard_ctx()));

        let pred = Host("blog.rust-lang.org");
        assert!(!pred.check(&req.guard_ctx()));

        let pred = Host("blog.rust-lang.org").scheme("https");
        assert!(!pred.check(&req.guard_ctx()));

        let pred = Host("crates.io").scheme("https");
        assert!(!pred.check(&req.guard_ctx()));

        let pred = Host("localhost");
        assert!(!pred.check(&req.guard_ctx()));
    }

    #[test]
    fn test_host_without_header() {
        let req = TestRequest::default()
            .uri("www.rust-lang.org")
            .to_srv_request();

        let pred = Host("www.rust-lang.org");
        assert!(pred.check(&req.guard_ctx()));

        let pred = Host("www.rust-lang.org").scheme("https");
        assert!(pred.check(&req.guard_ctx()));

        let pred = Host("blog.rust-lang.org");
        assert!(!pred.check(&req.guard_ctx()));

        let pred = Host("blog.rust-lang.org").scheme("https");
        assert!(!pred.check(&req.guard_ctx()));

        let pred = Host("crates.io");
        assert!(!pred.check(&req.guard_ctx()));

        let pred = Host("localhost");
        assert!(!pred.check(&req.guard_ctx()));
    }

    #[test]
    fn test_methods() {
        let req = TestRequest::default().to_srv_request();
        let req2 = TestRequest::default().method(Method::POST).to_srv_request();

        assert!(Get().check(&req.guard_ctx()));
        assert!(!Get().check(&req2.guard_ctx()));
        assert!(Post().check(&req2.guard_ctx()));
        assert!(!Post().check(&req.guard_ctx()));

        let r = TestRequest::default().method(Method::PUT).to_srv_request();
        assert!(Put().check(&r.guard_ctx()));
        assert!(!Put().check(&req.guard_ctx()));

        let r = TestRequest::default()
            .method(Method::DELETE)
            .to_srv_request();
        assert!(Delete().check(&r.guard_ctx()));
        assert!(!Delete().check(&req.guard_ctx()));

        let r = TestRequest::default().method(Method::HEAD).to_srv_request();
        assert!(Head().check(&r.guard_ctx()));
        assert!(!Head().check(&req.guard_ctx()));

        let r = TestRequest::default()
            .method(Method::OPTIONS)
            .to_srv_request();
        assert!(Options().check(&r.guard_ctx()));
        assert!(!Options().check(&req.guard_ctx()));

        let r = TestRequest::default()
            .method(Method::CONNECT)
            .to_srv_request();
        assert!(Connect().check(&r.guard_ctx()));
        assert!(!Connect().check(&req.guard_ctx()));

        let r = TestRequest::default()
            .method(Method::PATCH)
            .to_srv_request();
        assert!(Patch().check(&r.guard_ctx()));
        assert!(!Patch().check(&req.guard_ctx()));

        let r = TestRequest::default()
            .method(Method::TRACE)
            .to_srv_request();
        assert!(Trace().check(&r.guard_ctx()));
        assert!(!Trace().check(&req.guard_ctx()));
    }

    #[test]
    fn test_preds() {
        let r = TestRequest::default()
            .method(Method::TRACE)
            .to_srv_request();

        assert!(Not(Get()).check(&r.guard_ctx()));
        assert!(!Not(Trace()).check(&r.guard_ctx()));

        assert!(All(Trace()).and(Trace()).check(&r.guard_ctx()));
        assert!(!All(Get()).and(Trace()).check(&r.guard_ctx()));

        assert!(Any(Get()).or(Trace()).check(&r.guard_ctx()));
        assert!(!Any(Get()).or(Get()).check(&r.guard_ctx()));
    }
}
