//! Route guards.
//!
//! Guards are used during routing to help select a matching service or handler using some aspect of
//! the request; though guards should not be used for path matching since it is a built-in function
//! of the Actix Web router.
//!
//! Guards can be used on [`Scope`]s, [`Resource`]s, [`Route`]s, and other custom services.
//!
//! Fundamentally, a guard is a predicate function that receives a reference to a request context
//! object and returns a boolean; true if the request _should_ be handled by the guarded service
//! or handler. This interface is defined by the [`Guard`] trait.
//!
//! Commonly-used guards are provided in this module as well as way of creating a guard from a
//! closure ([`fn_guard`]). The [`Not`], [`Any`], and [`All`] guards are noteworthy, as they can be
//! used to compose other guards in a more flexible and semantic way than calling `.guard(...)` on
//! services multiple times (which might have different combining behavior than you want).
//!
//! Guards can not modify anything about the request. However, it is possible to store extra
//! attributes in the request-local data container obtained with [`GuardContext::req_data_mut`].
//!
//! Guards can prevent resource definitions from overlapping (resulting in some inaccessible routes)
//! where they otherwise would when only considering paths. See the virtual hosting example below.
//!
//! # Examples
//! In the following code, the `/guarded` resource has one defined route whose handler will only be
//! called if the request method is `POST` and there is a request header with name and value equal
//! to `x-guarded` and `secret`, respectively.
//! ```
//! use actix_web::{web, http::Method, guard, App, HttpResponse};
//!
//! web::resource("/guarded").route(
//!     web::route()
//!         .guard(guard::Any(guard::Get()).or(guard::Post()))
//!         .guard(guard::Header("x-guarded", "secret"))
//!         .to(|| HttpResponse::Ok())
//! );
//! ```
//!
//! Guards can be used to set up some form of [virtual hosting] within a single app.
//! Overlapping scope prefixes are usually discouraged, but when combined with non-overlapping guard
//! definitions they become safe to use in this way. Without these host guards, only routes under
//! the first-to-be-defined scope would be accessible. You can test this locally using `127.0.0.1`
//! and `localhost` as the `Host` guards.
//! ```
//! use actix_web::{web, http::Method, guard, App, HttpResponse};
//!
//! App::new()
//!     .service(
//!         web::scope("")
//!             .guard(guard::Host("www.rust-lang.org"))
//!             .default_service(web::to(|| HttpResponse::Ok().body("marketing site"))),
//!     )
//!     .service(
//!         web::scope("")
//!             .guard(guard::Host("play.rust-lang.org"))
//!             .default_service(web::to(|| HttpResponse::Ok().body("playground frontend"))),
//!     );
//! ```
//!
//! [`Scope`]: crate::Scope::guard()
//! [`Resource`]: crate::Resource::guard()
//! [`Route`]: crate::Route::guard()
//! [virtual hosting]: https://en.wikipedia.org/wiki/Virtual_hosting

use std::{
    cell::{Ref, RefMut},
    convert::TryFrom,
    rc::Rc,
};

use actix_http::{header, uri::Uri, Extensions, Method as HttpMethod, RequestHead};

use crate::service::ServiceRequest;

/// Provides access to request parts that are useful during routing.
#[derive(Debug)]
pub struct GuardContext<'a> {
    pub(crate) req: &'a ServiceRequest,
}

impl<'a> GuardContext<'a> {
    /// Returns reference to the request head.
    #[inline]
    pub fn head(&self) -> &RequestHead {
        self.req.head()
    }

    /// Returns reference to the request-local data container.
    #[inline]
    pub fn req_data(&self) -> Ref<'a, Extensions> {
        self.req.req_data()
    }

    /// Returns mutable reference to the request-local data container.
    #[inline]
    pub fn req_data_mut(&self) -> RefMut<'a, Extensions> {
        self.req.req_data_mut()
    }
}

/// Interface for routing guards.
///
/// See [module level documentation](self) for more.
pub trait Guard {
    /// Returns true if predicate condition is met for a given request.
    fn check(&self, ctx: &GuardContext<'_>) -> bool;
}

impl Guard for Rc<dyn Guard> {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        (**self).check(ctx)
    }
}

/// Creates a guard using the given function.
///
/// # Examples
/// ```
/// use actix_web::{guard, web, HttpResponse};
///
/// web::route()
///     .guard(guard::fn_guard(|ctx| {
///         ctx.head().headers().contains_key("content-type")
///     }))
///     .to(|| HttpResponse::Ok());
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
/// # Examples
/// The handler below will be called for either request method `GET` or `POST`.
/// ```
/// use actix_web::{web, guard, App, HttpResponse};
///
/// web::route()
///     .guard(
///         guard::Any(guard::Get())
///             .or(guard::Post()))
///     .to(|| HttpResponse::Ok());
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

/// Creates a guard that matches if all of the supplied guards.
///
/// # Examples
/// The handler below will only be called if the request method is `GET` **and** the specified
/// header name and value match exactly.
/// ```
/// use actix_web::{guard, web, HttpResponse};
///
/// web::route()
///     .guard(
///         guard::All(guard::Get())
///             .and(guard::Header("accept", "text/plain"))
///     )
///     .to(|| HttpResponse::Ok());
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

/// Wraps a guard and inverts the outcome of it's `Guard` implementation.
///
/// # Examples
/// The handler below will be called for any request method apart from `GET`.
/// ```
/// use actix_web::{guard, web, HttpResponse};
///
/// web::route()
///     .guard(guard::Not(guard::Get()))
///     .to(|| HttpResponse::Ok());
/// ```
pub struct Not<G>(pub G);

impl<G: Guard> Guard for Not<G> {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        !self.0.check(ctx)
    }
}

/// Predicate to match specified HTTP method.
#[allow(non_snake_case)]
pub fn Method(method: HttpMethod) -> impl Guard {
    MethodGuard(method)
}

/// HTTP method guard.
struct MethodGuard(HttpMethod);

impl Guard for MethodGuard {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        ctx.head().method == self.0
    }
}

macro_rules! method_guard {
    ($method_fn:ident, $method_const:ident) => {
        paste::paste! {
            #[doc = " Creates a guard that matches the `" $method_const "` request method."]
            ///
            /// # Examples
            #[doc = " The route in this example will only respond to `" $method_const "` requests."]
            /// ```
            /// use actix_web::{guard, web, HttpResponse};
            ///
            /// web::route()
            #[doc = "     .guard(guard::" $method_fn "())"]
            ///     .to(|| HttpResponse::Ok());
            /// ```
            #[allow(non_snake_case)]
            pub fn $method_fn() -> impl Guard {
                MethodGuard(HttpMethod::$method_const)
            }
        }
    };
}

method_guard!(Get, GET);
method_guard!(Post, POST);
method_guard!(Put, PUT);
method_guard!(Delete, DELETE);
method_guard!(Head, HEAD);
method_guard!(Options, OPTIONS);
method_guard!(Connect, CONNECT);
method_guard!(Patch, PATCH);
method_guard!(Trace, TRACE);

/// Creates a guard that matches if request contains given header name and value.
#[allow(non_snake_case)]
pub fn Header(name: &'static str, value: &'static str) -> impl Guard {
    HeaderGuard(
        header::HeaderName::try_from(name).unwrap(),
        header::HeaderValue::from_static(value),
    )
}

struct HeaderGuard(header::HeaderName, header::HeaderValue);

impl Guard for HeaderGuard {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        if let Some(val) = ctx.head().headers.get(&self.0) {
            return val == self.1;
        }

        false
    }
}

/// Creates a guard that matches requests targetting a specific host.
///
/// # Matching Host
/// This guard will:
/// - match against the `Host` header, if present;
/// - fall-back to matching against the request target's host, if present;
/// - return false if host cannot be determined;
///
/// # Matching Scheme
/// Optionally, this guard can match against the host's scheme. Set the scheme for matching using
/// `Host(host).scheme(protocol)`. If the request's scheme cannot be determined, it will not prevent
/// the guard from matching successfully.
///
/// # Examples
/// The [module-level documentation](self) has an example of virtual hosting using `Host` guards.
///
/// The example below additionally guards on the host URI's scheme. This could allow routing to
/// different handlers for `http:` vs `https:` visitors; to redirect, for example.
/// ```
/// use actix_web::{web, guard::Host, HttpResponse};
///
/// web::scope("/admin")
///     .guard(Host("admin.rust-lang.org").scheme("https"))
///     .default_service(web::to(|| HttpResponse::Ok().body("admin connection is secure")));
/// ```
#[allow(non_snake_case)]
pub fn Host(host: impl AsRef<str>) -> HostGuard {
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
        .and_then(|host| host.parse().ok())
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
        // parse host URI from header or request target
        let req_host_uri = match get_host_uri(ctx.head()) {
            Some(uri) => uri,

            // no match if host cannot be determined
            None => return false,
        };

        match req_host_uri.host() {
            // fall through to scheme checks
            Some(uri_host) if self.host == uri_host => {}

            // Either:
            // - request's host does not match guard's host;
            // - It was possible that the parsed URI from request target did not contain a host.
            _ => return false,
        }

        if let Some(ref scheme) = self.scheme {
            if let Some(ref req_host_uri_scheme) = req_host_uri.scheme_str() {
                return scheme == req_host_uri_scheme;
            }

            // TODO: is the the correct behavior?
            // falls through if scheme cannot be determined
        }

        // all conditions passed
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

    #[test]
    fn not_guard_reflexive() {
        let req = TestRequest::default().to_srv_request();

        let get = Get();
        assert!(get.check(&req.guard_ctx()));

        let not_get = Not(get);
        assert!(!not_get.check(&req.guard_ctx()));

        let not_not_get = Not(not_get);
        assert!(not_not_get.check(&req.guard_ctx()));
    }
}
