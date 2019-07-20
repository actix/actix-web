//! Route match guards.
//!
//! Guards are one of the way how actix-web router chooses
//! handler service. In essence it just function that accepts
//! reference to a `RequestHead` instance and returns boolean.
//! It is possible to add guards to *scopes*, *resources*
//! and *routes*. Actix provide several guards by default, like various
//! http methods, header, etc. To become a guard, type must implement `Guard`
//! trait. Simple functions coulds guards as well.
//!
//! Guard can not modify request object. But it is possible to
//! to store extra attributes on a request by using `Extensions` container.
//! Extensions container available via `RequestHead::extensions()` method.
//!
//! ```rust
//! use actix_web::{web, http, dev, guard, App, HttpResponse};
//!
//! fn main() {
//!     App::new().service(web::resource("/index.html").route(
//!         web::route()
//!              .guard(guard::Post())
//!              .guard(guard::fn_guard(|head| head.method == http::Method::GET))
//!              .to(|| HttpResponse::MethodNotAllowed()))
//!     );
//! }
//! ```

#![allow(non_snake_case)]
use actix_http::http::{self, header, uri::Uri, HttpTryFrom};
use actix_http::RequestHead;

/// Trait defines resource guards. Guards are used for routes selection.
///
/// Guard can not modify request object. But it is possible to
/// to store extra attributes on request by using `Extensions` container,
/// Extensions container available via `RequestHead::extensions()` method.
pub trait Guard {
    /// Check if request matches predicate
    fn check(&self, request: &RequestHead) -> bool;
}

/// Create guard object for supplied function.
///
/// ```rust
/// use actix_web::{guard, web, App, HttpResponse};
///
/// fn main() {
///     App::new().service(web::resource("/index.html").route(
///         web::route()
///             .guard(
///                 guard::fn_guard(
///                     |req| req.headers()
///                              .contains_key("content-type")))
///             .to(|| HttpResponse::MethodNotAllowed()))
///     );
/// }
/// ```
pub fn fn_guard<F>(f: F) -> impl Guard
where
    F: Fn(&RequestHead) -> bool,
{
    FnGuard(f)
}

struct FnGuard<F: Fn(&RequestHead) -> bool>(F);

impl<F> Guard for FnGuard<F>
where
    F: Fn(&RequestHead) -> bool,
{
    fn check(&self, head: &RequestHead) -> bool {
        (self.0)(head)
    }
}

impl<F> Guard for F
where
    F: Fn(&RequestHead) -> bool,
{
    fn check(&self, head: &RequestHead) -> bool {
        (self)(head)
    }
}

/// Return guard that matches if any of supplied guards.
///
/// ```rust
/// use actix_web::{web, guard, App, HttpResponse};
///
/// fn main() {
///     App::new().service(web::resource("/index.html").route(
///         web::route()
///              .guard(guard::Any(guard::Get()).or(guard::Post()))
///              .to(|| HttpResponse::MethodNotAllowed()))
///     );
/// }
/// ```
pub fn Any<F: Guard + 'static>(guard: F) -> AnyGuard {
    AnyGuard(vec![Box::new(guard)])
}

/// Matches if any of supplied guards matche.
pub struct AnyGuard(Vec<Box<dyn Guard>>);

impl AnyGuard {
    /// Add guard to a list of guards to check
    pub fn or<F: Guard + 'static>(mut self, guard: F) -> Self {
        self.0.push(Box::new(guard));
        self
    }
}

impl Guard for AnyGuard {
    fn check(&self, req: &RequestHead) -> bool {
        for p in &self.0 {
            if p.check(req) {
                return true;
            }
        }
        false
    }
}

/// Return guard that matches if all of the supplied guards.
///
/// ```rust
/// use actix_web::{guard, web, App, HttpResponse};
///
/// fn main() {
///     App::new().service(web::resource("/index.html").route(
///         web::route()
///             .guard(
///                 guard::All(guard::Get()).and(guard::Header("content-type", "text/plain")))
///             .to(|| HttpResponse::MethodNotAllowed()))
///     );
/// }
/// ```
pub fn All<F: Guard + 'static>(guard: F) -> AllGuard {
    AllGuard(vec![Box::new(guard)])
}

/// Matches if all of supplied guards.
pub struct AllGuard(Vec<Box<dyn Guard>>);

impl AllGuard {
    /// Add new guard to the list of guards to check
    pub fn and<F: Guard + 'static>(mut self, guard: F) -> Self {
        self.0.push(Box::new(guard));
        self
    }
}

impl Guard for AllGuard {
    fn check(&self, request: &RequestHead) -> bool {
        for p in &self.0 {
            if !p.check(request) {
                return false;
            }
        }
        true
    }
}

/// Return guard that matches if supplied guard does not match.
pub fn Not<F: Guard + 'static>(guard: F) -> NotGuard {
    NotGuard(Box::new(guard))
}

#[doc(hidden)]
pub struct NotGuard(Box<dyn Guard>);

impl Guard for NotGuard {
    fn check(&self, request: &RequestHead) -> bool {
        !self.0.check(request)
    }
}

/// Http method guard
#[doc(hidden)]
pub struct MethodGuard(http::Method);

impl Guard for MethodGuard {
    fn check(&self, request: &RequestHead) -> bool {
        request.method == self.0
    }
}

/// Guard to match *GET* http method
pub fn Get() -> MethodGuard {
    MethodGuard(http::Method::GET)
}

/// Predicate to match *POST* http method
pub fn Post() -> MethodGuard {
    MethodGuard(http::Method::POST)
}

/// Predicate to match *PUT* http method
pub fn Put() -> MethodGuard {
    MethodGuard(http::Method::PUT)
}

/// Predicate to match *DELETE* http method
pub fn Delete() -> MethodGuard {
    MethodGuard(http::Method::DELETE)
}

/// Predicate to match *HEAD* http method
pub fn Head() -> MethodGuard {
    MethodGuard(http::Method::HEAD)
}

/// Predicate to match *OPTIONS* http method
pub fn Options() -> MethodGuard {
    MethodGuard(http::Method::OPTIONS)
}

/// Predicate to match *CONNECT* http method
pub fn Connect() -> MethodGuard {
    MethodGuard(http::Method::CONNECT)
}

/// Predicate to match *PATCH* http method
pub fn Patch() -> MethodGuard {
    MethodGuard(http::Method::PATCH)
}

/// Predicate to match *TRACE* http method
pub fn Trace() -> MethodGuard {
    MethodGuard(http::Method::TRACE)
}

/// Predicate to match specified http method
pub fn Method(method: http::Method) -> MethodGuard {
    MethodGuard(method)
}

/// Return predicate that matches if request contains specified header and
/// value.
pub fn Header(name: &'static str, value: &'static str) -> HeaderGuard {
    HeaderGuard(
        header::HeaderName::try_from(name).unwrap(),
        header::HeaderValue::from_static(value),
    )
}

#[doc(hidden)]
pub struct HeaderGuard(header::HeaderName, header::HeaderValue);

impl Guard for HeaderGuard {
    fn check(&self, req: &RequestHead) -> bool {
        if let Some(val) = req.headers.get(&self.0) {
            return val == self.1;
        }
        false
    }
}

/// Return predicate that matches if request contains specified Host name.
///
/// ```rust,ignore
/// # extern crate actix_web;
/// use actix_web::{guard::Host, App, HttpResponse};
///
/// fn main() {
///     App::new().resource("/index.html", |r| {
///         r.route()
///             .guard(Host("www.rust-lang.org"))
///             .f(|_| HttpResponse::MethodNotAllowed())
///     });
/// }
/// ```
pub fn Host<H: AsRef<str>>(host: H) -> HostGuard {
    HostGuard(host.as_ref().to_string(), None)
}

fn get_host_uri(req: &RequestHead) -> Option<Uri> {
    use core::str::FromStr;
    let host_value = req.headers.get(header::HOST)?;
    let host = host_value.to_str().ok()?;
    let uri = Uri::from_str(host).ok()?;
    Some(uri)
}

#[doc(hidden)]
pub struct HostGuard(String, Option<String>);

impl HostGuard {
    /// Set request scheme to match
    pub fn scheme<H: AsRef<str>>(mut self, scheme: H) -> HostGuard {
        self.1 = Some(scheme.as_ref().to_string());
        self
    }
}

impl Guard for HostGuard {
    fn check(&self, req: &RequestHead) -> bool {
        let req_host_uri = if let Some(uri) = get_host_uri(req) {
            uri
        } else {
            return false;
        };

        if let Some(uri_host) = req_host_uri.host() {
            if self.0 != uri_host {
                return false;
            }
        } else {
            return false;
        }

        if let Some(ref scheme) = self.1 {
            if let Some(ref req_host_uri_scheme) = req_host_uri.scheme_str() {
                return scheme == req_host_uri_scheme;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use actix_http::http::{header, Method};

    use super::*;
    use crate::test::TestRequest;

    #[test]
    fn test_header() {
        let req = TestRequest::with_header(header::TRANSFER_ENCODING, "chunked")
            .to_http_request();

        let pred = Header("transfer-encoding", "chunked");
        assert!(pred.check(req.head()));

        let pred = Header("transfer-encoding", "other");
        assert!(!pred.check(req.head()));

        let pred = Header("content-type", "other");
        assert!(!pred.check(req.head()));
    }

    #[test]
    fn test_host() {
        let req = TestRequest::default()
            .header(
                header::HOST,
                header::HeaderValue::from_static("www.rust-lang.org"),
            )
            .to_http_request();

        let pred = Host("www.rust-lang.org");
        assert!(pred.check(req.head()));

        let pred = Host("www.rust-lang.org").scheme("https");
        assert!(pred.check(req.head()));

        let pred = Host("blog.rust-lang.org");
        assert!(!pred.check(req.head()));

        let pred = Host("blog.rust-lang.org").scheme("https");
        assert!(!pred.check(req.head()));

        let pred = Host("crates.io");
        assert!(!pred.check(req.head()));

        let pred = Host("localhost");
        assert!(!pred.check(req.head()));
    }

    #[test]
    fn test_host_scheme() {
        let req = TestRequest::default()
            .header(
                header::HOST,
                header::HeaderValue::from_static("https://www.rust-lang.org"),
            )
            .to_http_request();

        let pred = Host("www.rust-lang.org").scheme("https");
        assert!(pred.check(req.head()));

        let pred = Host("www.rust-lang.org");
        assert!(pred.check(req.head()));

        let pred = Host("www.rust-lang.org").scheme("http");
        assert!(!pred.check(req.head()));

        let pred = Host("blog.rust-lang.org");
        assert!(!pred.check(req.head()));

        let pred = Host("blog.rust-lang.org").scheme("https");
        assert!(!pred.check(req.head()));

        let pred = Host("crates.io").scheme("https");
        assert!(!pred.check(req.head()));

        let pred = Host("localhost");
        assert!(!pred.check(req.head()));
    }

    #[test]
    fn test_methods() {
        let req = TestRequest::default().to_http_request();
        let req2 = TestRequest::default()
            .method(Method::POST)
            .to_http_request();

        assert!(Get().check(req.head()));
        assert!(!Get().check(req2.head()));
        assert!(Post().check(req2.head()));
        assert!(!Post().check(req.head()));

        let r = TestRequest::default().method(Method::PUT).to_http_request();
        assert!(Put().check(r.head()));
        assert!(!Put().check(req.head()));

        let r = TestRequest::default()
            .method(Method::DELETE)
            .to_http_request();
        assert!(Delete().check(r.head()));
        assert!(!Delete().check(req.head()));

        let r = TestRequest::default()
            .method(Method::HEAD)
            .to_http_request();
        assert!(Head().check(r.head()));
        assert!(!Head().check(req.head()));

        let r = TestRequest::default()
            .method(Method::OPTIONS)
            .to_http_request();
        assert!(Options().check(r.head()));
        assert!(!Options().check(req.head()));

        let r = TestRequest::default()
            .method(Method::CONNECT)
            .to_http_request();
        assert!(Connect().check(r.head()));
        assert!(!Connect().check(req.head()));

        let r = TestRequest::default()
            .method(Method::PATCH)
            .to_http_request();
        assert!(Patch().check(r.head()));
        assert!(!Patch().check(req.head()));

        let r = TestRequest::default()
            .method(Method::TRACE)
            .to_http_request();
        assert!(Trace().check(r.head()));
        assert!(!Trace().check(req.head()));
    }

    #[test]
    fn test_preds() {
        let r = TestRequest::default()
            .method(Method::TRACE)
            .to_http_request();

        assert!(Not(Get()).check(r.head()));
        assert!(!Not(Trace()).check(r.head()));

        assert!(All(Trace()).and(Trace()).check(r.head()));
        assert!(!All(Get()).and(Trace()).check(r.head()));

        assert!(Any(Get()).or(Trace()).check(r.head()));
        assert!(!Any(Get()).or(Get()).check(r.head()));
    }
}
