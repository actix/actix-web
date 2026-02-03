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
//! Commonly-used guards are provided in this module as well as a way of creating a guard from a
//! closure ([`fn_guard`]). The [`Not`], [`Any`], and [`All`] guards are noteworthy, as they can be
//! used to compose other guards in a more flexible and semantic way than calling `.guard(...)` on
//! services multiple times (which might have different combining behavior than you want).
//!
//! There are shortcuts for routes with method guards in the [`web`](crate::web) module:
//! [`web::get()`](crate::web::get), [`web::post()`](crate::web::post), etc. The routes created by
//! the following calls are equivalent:
//!
//! - `web::get()` (recommended form)
//! - `web::route().guard(guard::Get())`
//!
//! Guards can not modify anything about the request. However, it is possible to store extra
//! attributes in the request-local data container obtained with [`GuardContext::req_data_mut`].
//!
//! Guards can prevent resource definitions from overlapping which, when only considering paths,
//! would result in inaccessible routes. See the [`Host`] guard for an example of virtual hosting.
//!
//! # Examples
//!
//! In the following code, the `/guarded` resource has one defined route whose handler will only be
//! called if the request method is GET or POST and there is a `x-guarded` request header with value
//! equal to `secret`.
//!
//! ```
//! use actix_web::{web, http::Method, guard, HttpResponse};
//!
//! web::resource("/guarded").route(
//!     web::route()
//!         .guard(guard::Any(guard::Get()).or(guard::Post()))
//!         .guard(guard::Header("x-guarded", "secret"))
//!         .to(|| HttpResponse::Ok())
//! );
//! ```
//!
//! [`Scope`]: crate::Scope::guard()
//! [`Resource`]: crate::Resource::guard()
//! [`Route`]: crate::Route::guard()

use std::{
    cell::{Ref, RefMut},
    rc::Rc,
};

use actix_http::{header, Extensions, Method as HttpMethod, RequestHead};

use crate::{http::header::Header, service::ServiceRequest, HttpMessage as _};

mod acceptable;
mod host;

pub use self::{
    acceptable::Acceptable,
    host::{Host, HostGuard},
};

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

    /// Returns reference to the request-local data/extensions container.
    #[inline]
    pub fn req_data(&self) -> Ref<'a, Extensions> {
        self.req.extensions()
    }

    /// Returns mutable reference to the request-local data/extensions container.
    #[inline]
    pub fn req_data_mut(&self) -> RefMut<'a, Extensions> {
        self.req.extensions_mut()
    }

    /// Extracts a typed header from the request.
    ///
    /// Returns `None` if parsing `H` fails.
    ///
    /// # Examples
    /// ```
    /// use actix_web::{guard::fn_guard, http::header};
    ///
    /// let image_accept_guard = fn_guard(|ctx| {
    ///     match ctx.header::<header::Accept>() {
    ///         Some(hdr) => hdr.preference() == "image/*",
    ///         None => false,
    ///     }
    /// });
    /// ```
    #[inline]
    pub fn header<H: Header>(&self) -> Option<H> {
        H::parse(self.req).ok()
    }

    /// Counterpart to [HttpRequest::app_data](crate::HttpRequest::app_data).
    #[inline]
    pub fn app_data<T: 'static>(&self) -> Option<&T> {
        self.req.app_data()
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

/// Creates a guard that matches if any added guards match.
///
/// # Examples
/// The handler below will be called for either request method `GET` or `POST`.
/// ```
/// use actix_web::{web, guard, HttpResponse};
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

/// A collection of guards that match if the disjunction of their `check` outcomes is true.
///
/// That is, only one contained guard needs to match in order for the aggregate guard to match.
///
/// Construct an `AnyGuard` using [`Any`].
pub struct AnyGuard {
    guards: Vec<Box<dyn Guard>>,
}

impl AnyGuard {
    /// Adds new guard to the collection of guards to check.
    pub fn or<F: Guard + 'static>(mut self, guard: F) -> Self {
        self.guards.push(Box::new(guard));
        self
    }
}

impl Guard for AnyGuard {
    #[inline]
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        for guard in &self.guards {
            if guard.check(ctx) {
                return true;
            }
        }

        false
    }
}

/// Creates a guard that matches if all added guards match.
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

/// A collection of guards that match if the conjunction of their `check` outcomes is true.
///
/// That is, **all** contained guard needs to match in order for the aggregate guard to match.
///
/// Construct an `AllGuard` using [`All`].
pub struct AllGuard {
    guards: Vec<Box<dyn Guard>>,
}

impl AllGuard {
    /// Adds new guard to the collection of guards to check.
    pub fn and<F: Guard + 'static>(mut self, guard: F) -> Self {
        self.guards.push(Box::new(guard));
        self
    }
}

impl Guard for AllGuard {
    #[inline]
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        for guard in &self.guards {
            if !guard.check(ctx) {
                return false;
            }
        }

        true
    }
}

/// Wraps a guard and inverts the outcome of its `Guard` implementation.
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
    #[inline]
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        !self.0.check(ctx)
    }
}

/// Creates a guard that matches a specified HTTP method.
#[allow(non_snake_case)]
pub fn Method(method: HttpMethod) -> impl Guard {
    MethodGuard(method)
}

#[derive(Debug, Clone)]
pub(crate) struct RegisteredMethods(pub(crate) Vec<HttpMethod>);

/// HTTP method guard.
#[derive(Debug)]
pub(crate) struct MethodGuard(HttpMethod);

impl Guard for MethodGuard {
    fn check(&self, ctx: &GuardContext<'_>) -> bool {
        let registered = ctx.req_data_mut().remove::<RegisteredMethods>();

        if let Some(mut methods) = registered {
            methods.0.push(self.0.clone());
            ctx.req_data_mut().insert(methods);
        } else {
            ctx.req_data_mut()
                .insert(RegisteredMethods(vec![self.0.clone()]));
        }

        ctx.head().method == self.0
    }
}

macro_rules! method_guard {
    ($method_fn:ident, $method_const:ident) => {
        #[doc = concat!("Creates a guard that matches the `", stringify!($method_const), "` request method.")]
        ///
        /// # Examples
        #[doc = concat!("The route in this example will only respond to `", stringify!($method_const), "` requests.")]
        /// ```
        /// use actix_web::{guard, web, HttpResponse};
        ///
        /// web::route()
        #[doc = concat!("    .guard(guard::", stringify!($method_fn), "())")]
        ///     .to(|| HttpResponse::Ok());
        /// ```
        #[allow(non_snake_case)]
        pub fn $method_fn() -> impl Guard {
            MethodGuard(HttpMethod::$method_const)
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
///
/// # Examples
/// The handler below will be called when the request contains an `x-guarded` header with value
/// equal to `secret`.
/// ```
/// use actix_web::{guard, web, HttpResponse};
///
/// web::route()
///     .guard(guard::Header("x-guarded", "secret"))
///     .to(|| HttpResponse::Ok());
/// ```
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

#[cfg(test)]
mod tests {
    use actix_http::Method;

    use super::*;
    use crate::test::TestRequest;

    #[test]
    fn header_match() {
        let req = TestRequest::default()
            .insert_header((header::TRANSFER_ENCODING, "chunked"))
            .to_srv_request();

        let hdr = Header("transfer-encoding", "chunked");
        assert!(hdr.check(&req.guard_ctx()));

        let hdr = Header("transfer-encoding", "other");
        assert!(!hdr.check(&req.guard_ctx()));

        let hdr = Header("content-type", "chunked");
        assert!(!hdr.check(&req.guard_ctx()));

        let hdr = Header("content-type", "other");
        assert!(!hdr.check(&req.guard_ctx()));
    }

    #[test]
    fn method_guards() {
        let get_req = TestRequest::get().to_srv_request();
        let post_req = TestRequest::post().to_srv_request();

        assert!(Get().check(&get_req.guard_ctx()));
        assert!(!Get().check(&post_req.guard_ctx()));

        assert!(Post().check(&post_req.guard_ctx()));
        assert!(!Post().check(&get_req.guard_ctx()));

        let req = TestRequest::put().to_srv_request();
        assert!(Put().check(&req.guard_ctx()));
        assert!(!Put().check(&get_req.guard_ctx()));

        let req = TestRequest::patch().to_srv_request();
        assert!(Patch().check(&req.guard_ctx()));
        assert!(!Patch().check(&get_req.guard_ctx()));

        let r = TestRequest::delete().to_srv_request();
        assert!(Delete().check(&r.guard_ctx()));
        assert!(!Delete().check(&get_req.guard_ctx()));

        let req = TestRequest::default().method(Method::HEAD).to_srv_request();
        assert!(Head().check(&req.guard_ctx()));
        assert!(!Head().check(&get_req.guard_ctx()));

        let req = TestRequest::default()
            .method(Method::OPTIONS)
            .to_srv_request();
        assert!(Options().check(&req.guard_ctx()));
        assert!(!Options().check(&get_req.guard_ctx()));

        let req = TestRequest::default()
            .method(Method::CONNECT)
            .to_srv_request();
        assert!(Connect().check(&req.guard_ctx()));
        assert!(!Connect().check(&get_req.guard_ctx()));

        let req = TestRequest::default()
            .method(Method::TRACE)
            .to_srv_request();
        assert!(Trace().check(&req.guard_ctx()));
        assert!(!Trace().check(&get_req.guard_ctx()));
    }

    #[test]
    fn aggregate_any() {
        let req = TestRequest::default()
            .method(Method::TRACE)
            .to_srv_request();

        assert!(Any(Trace()).check(&req.guard_ctx()));
        assert!(Any(Trace()).or(Get()).check(&req.guard_ctx()));
        assert!(!Any(Get()).or(Get()).check(&req.guard_ctx()));
    }

    #[test]
    fn aggregate_all() {
        let req = TestRequest::default()
            .method(Method::TRACE)
            .to_srv_request();

        assert!(All(Trace()).check(&req.guard_ctx()));
        assert!(All(Trace()).and(Trace()).check(&req.guard_ctx()));
        assert!(!All(Trace()).and(Get()).check(&req.guard_ctx()));
    }

    #[test]
    fn nested_not() {
        let req = TestRequest::default().to_srv_request();

        let get = Get();
        assert!(get.check(&req.guard_ctx()));

        let not_get = Not(get);
        assert!(!not_get.check(&req.guard_ctx()));

        let not_not_get = Not(not_get);
        assert!(not_not_get.check(&req.guard_ctx()));
    }

    #[test]
    fn function_guard() {
        let domain = "rust-lang.org".to_owned();
        let guard = fn_guard(|ctx| ctx.head().uri.host().unwrap().ends_with(&domain));

        let req = TestRequest::default()
            .uri("blog.rust-lang.org")
            .to_srv_request();
        assert!(guard.check(&req.guard_ctx()));

        let req = TestRequest::default().uri("crates.io").to_srv_request();
        assert!(!guard.check(&req.guard_ctx()));
    }

    #[test]
    fn mega_nesting() {
        let guard = fn_guard(|ctx| All(Not(Any(Not(Trace())))).check(ctx));

        let req = TestRequest::default().to_srv_request();
        assert!(!guard.check(&req.guard_ctx()));

        let req = TestRequest::default()
            .method(Method::TRACE)
            .to_srv_request();
        assert!(guard.check(&req.guard_ctx()));
    }

    #[test]
    fn app_data() {
        const TEST_VALUE: u32 = 42;
        let guard = fn_guard(|ctx| dbg!(ctx.app_data::<u32>()) == Some(&TEST_VALUE));

        let req = TestRequest::default().app_data(TEST_VALUE).to_srv_request();
        assert!(guard.check(&req.guard_ctx()));

        let req = TestRequest::default()
            .app_data(TEST_VALUE * 2)
            .to_srv_request();
        assert!(!guard.check(&req.guard_ctx()));
    }
}
