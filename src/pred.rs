//! Route match predicates
#![allow(non_snake_case)]
use std::marker::PhantomData;

use http;
use http::{header, HttpTryFrom};
use server::message::Request;

/// Trait defines resource route predicate.
/// Predicate can modify request object. It is also possible to
/// to store extra attributes on request by using `Extensions` container,
/// Extensions container available via `HttpRequest::extensions()` method.
pub trait Predicate<S> {
    /// Check if request matches predicate
    fn check(&self, &Request, &S) -> bool;
}

/// Return predicate that matches if any of supplied predicate matches.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{pred, App, HttpResponse};
///
/// fn main() {
///     App::new().resource("/index.html", |r| {
///         r.route()
///             .filter(pred::Any(pred::Get()).or(pred::Post()))
///             .f(|r| HttpResponse::MethodNotAllowed())
///     });
/// }
/// ```
pub fn Any<S: 'static, P: Predicate<S> + 'static>(pred: P) -> AnyPredicate<S> {
    AnyPredicate(vec![Box::new(pred)])
}

/// Matches if any of supplied predicate matches.
pub struct AnyPredicate<S>(Vec<Box<Predicate<S>>>);

impl<S> AnyPredicate<S> {
    /// Add new predicate to list of predicates to check
    pub fn or<P: Predicate<S> + 'static>(mut self, pred: P) -> Self {
        self.0.push(Box::new(pred));
        self
    }
}

impl<S: 'static> Predicate<S> for AnyPredicate<S> {
    fn check(&self, req: &Request, state: &S) -> bool {
        for p in &self.0 {
            if p.check(req, state) {
                return true;
            }
        }
        false
    }
}

/// Return predicate that matches if all of supplied predicate matches.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{pred, App, HttpResponse};
///
/// fn main() {
///     App::new().resource("/index.html", |r| {
///         r.route()
///             .filter(
///                 pred::All(pred::Get())
///                     .and(pred::Header("content-type", "text/plain")),
///             )
///             .f(|_| HttpResponse::MethodNotAllowed())
///     });
/// }
/// ```
pub fn All<S: 'static, P: Predicate<S> + 'static>(pred: P) -> AllPredicate<S> {
    AllPredicate(vec![Box::new(pred)])
}

/// Matches if all of supplied predicate matches.
pub struct AllPredicate<S>(Vec<Box<Predicate<S>>>);

impl<S> AllPredicate<S> {
    /// Add new predicate to list of predicates to check
    pub fn and<P: Predicate<S> + 'static>(mut self, pred: P) -> Self {
        self.0.push(Box::new(pred));
        self
    }
}

impl<S: 'static> Predicate<S> for AllPredicate<S> {
    fn check(&self, req: &Request, state: &S) -> bool {
        for p in &self.0 {
            if !p.check(req, state) {
                return false;
            }
        }
        true
    }
}

/// Return predicate that matches if supplied predicate does not match.
pub fn Not<S: 'static, P: Predicate<S> + 'static>(pred: P) -> NotPredicate<S> {
    NotPredicate(Box::new(pred))
}

#[doc(hidden)]
pub struct NotPredicate<S>(Box<Predicate<S>>);

impl<S: 'static> Predicate<S> for NotPredicate<S> {
    fn check(&self, req: &Request, state: &S) -> bool {
        !self.0.check(req, state)
    }
}

/// Http method predicate
#[doc(hidden)]
pub struct MethodPredicate<S>(http::Method, PhantomData<S>);

impl<S: 'static> Predicate<S> for MethodPredicate<S> {
    fn check(&self, req: &Request, _: &S) -> bool {
        *req.method() == self.0
    }
}

/// Predicate to match *GET* http method
pub fn Get<S: 'static>() -> MethodPredicate<S> {
    MethodPredicate(http::Method::GET, PhantomData)
}

/// Predicate to match *POST* http method
pub fn Post<S: 'static>() -> MethodPredicate<S> {
    MethodPredicate(http::Method::POST, PhantomData)
}

/// Predicate to match *PUT* http method
pub fn Put<S: 'static>() -> MethodPredicate<S> {
    MethodPredicate(http::Method::PUT, PhantomData)
}

/// Predicate to match *DELETE* http method
pub fn Delete<S: 'static>() -> MethodPredicate<S> {
    MethodPredicate(http::Method::DELETE, PhantomData)
}

/// Predicate to match *HEAD* http method
pub fn Head<S: 'static>() -> MethodPredicate<S> {
    MethodPredicate(http::Method::HEAD, PhantomData)
}

/// Predicate to match *OPTIONS* http method
pub fn Options<S: 'static>() -> MethodPredicate<S> {
    MethodPredicate(http::Method::OPTIONS, PhantomData)
}

/// Predicate to match *CONNECT* http method
pub fn Connect<S: 'static>() -> MethodPredicate<S> {
    MethodPredicate(http::Method::CONNECT, PhantomData)
}

/// Predicate to match *PATCH* http method
pub fn Patch<S: 'static>() -> MethodPredicate<S> {
    MethodPredicate(http::Method::PATCH, PhantomData)
}

/// Predicate to match *TRACE* http method
pub fn Trace<S: 'static>() -> MethodPredicate<S> {
    MethodPredicate(http::Method::TRACE, PhantomData)
}

/// Predicate to match specified http method
pub fn Method<S: 'static>(method: http::Method) -> MethodPredicate<S> {
    MethodPredicate(method, PhantomData)
}

/// Return predicate that matches if request contains specified header and
/// value.
pub fn Header<S: 'static>(
    name: &'static str, value: &'static str,
) -> HeaderPredicate<S> {
    HeaderPredicate(
        header::HeaderName::try_from(name).unwrap(),
        header::HeaderValue::from_static(value),
        PhantomData,
    )
}

#[doc(hidden)]
pub struct HeaderPredicate<S>(header::HeaderName, header::HeaderValue, PhantomData<S>);

impl<S: 'static> Predicate<S> for HeaderPredicate<S> {
    fn check(&self, req: &Request, _: &S) -> bool {
        if let Some(val) = req.headers().get(&self.0) {
            return val == self.1;
        }
        false
    }
}

/// Return predicate that matches if request contains specified Host name.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{pred, App, HttpResponse};
///
/// fn main() {
///     App::new().resource("/index.html", |r| {
///         r.route()
///             .filter(pred::Host("www.rust-lang.org"))
///             .f(|_| HttpResponse::MethodNotAllowed())
///     });
/// }
/// ```
pub fn Host<S: 'static, H: AsRef<str>>(host: H) -> HostPredicate<S> {
    HostPredicate(host.as_ref().to_string(), None, PhantomData)
}

#[doc(hidden)]
pub struct HostPredicate<S>(String, Option<String>, PhantomData<S>);

impl<S> HostPredicate<S> {
    /// Set reuest scheme to match
    pub fn scheme<H: AsRef<str>>(&mut self, scheme: H) {
        self.1 = Some(scheme.as_ref().to_string())
    }
}

impl<S: 'static> Predicate<S> for HostPredicate<S> {
    fn check(&self, req: &Request, _: &S) -> bool {
        let info = req.connection_info();
        if let Some(ref scheme) = self.1 {
            self.0 == info.host() && scheme == info.scheme()
        } else {
            self.0 == info.host()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{header, Method};
    use test::TestRequest;

    #[test]
    fn test_header() {
        let req = TestRequest::with_header(
            header::TRANSFER_ENCODING,
            header::HeaderValue::from_static("chunked"),
        ).finish();

        let pred = Header("transfer-encoding", "chunked");
        assert!(pred.check(&req, req.state()));

        let pred = Header("transfer-encoding", "other");
        assert!(!pred.check(&req, req.state()));

        let pred = Header("content-type", "other");
        assert!(!pred.check(&req, req.state()));
    }

    #[test]
    fn test_host() {
        let req = TestRequest::default()
            .header(
                header::HOST,
                header::HeaderValue::from_static("www.rust-lang.org"),
            )
            .finish();

        let pred = Host("www.rust-lang.org");
        assert!(pred.check(&req, req.state()));

        let pred = Host("localhost");
        assert!(!pred.check(&req, req.state()));
    }

    #[test]
    fn test_methods() {
        let req = TestRequest::default().finish();
        let req2 = TestRequest::default().method(Method::POST).finish();

        assert!(Get().check(&req, req.state()));
        assert!(!Get().check(&req2, req2.state()));
        assert!(Post().check(&req2, req2.state()));
        assert!(!Post().check(&req, req.state()));

        let r = TestRequest::default().method(Method::PUT).finish();
        assert!(Put().check(&r, r.state()));
        assert!(!Put().check(&req, req.state()));

        let r = TestRequest::default().method(Method::DELETE).finish();
        assert!(Delete().check(&r, r.state()));
        assert!(!Delete().check(&req, req.state()));

        let r = TestRequest::default().method(Method::HEAD).finish();
        assert!(Head().check(&r, r.state()));
        assert!(!Head().check(&req, req.state()));

        let r = TestRequest::default().method(Method::OPTIONS).finish();
        assert!(Options().check(&r, r.state()));
        assert!(!Options().check(&req, req.state()));

        let r = TestRequest::default().method(Method::CONNECT).finish();
        assert!(Connect().check(&r, r.state()));
        assert!(!Connect().check(&req, req.state()));

        let r = TestRequest::default().method(Method::PATCH).finish();
        assert!(Patch().check(&r, r.state()));
        assert!(!Patch().check(&req, req.state()));

        let r = TestRequest::default().method(Method::TRACE).finish();
        assert!(Trace().check(&r, r.state()));
        assert!(!Trace().check(&req, req.state()));
    }

    #[test]
    fn test_preds() {
        let r = TestRequest::default().method(Method::TRACE).finish();

        assert!(Not(Get()).check(&r, r.state()));
        assert!(!Not(Trace()).check(&r, r.state()));

        assert!(All(Trace()).and(Trace()).check(&r, r.state()));
        assert!(!All(Get()).and(Trace()).check(&r, r.state()));

        assert!(Any(Get()).or(Trace()).check(&r, r.state()));
        assert!(!Any(Get()).or(Get()).check(&r, r.state()));
    }
}
