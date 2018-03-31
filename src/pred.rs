//! Route match predicates
#![allow(non_snake_case)]
use std::marker::PhantomData;
use http;
use http::{header, HttpTryFrom};
use httpmessage::HttpMessage;
use httprequest::HttpRequest;

/// Trait defines resource route predicate.
/// Predicate can modify request object. It is also possible to
/// to store extra attributes on request by using `Extensions` container,
/// Extensions container available via `HttpRequest::extensions()` method.
pub trait Predicate<S> {

    /// Check if request matches predicate
    fn check(&self, &mut HttpRequest<S>) -> bool;

}

/// Return predicate that matches if any of supplied predicate matches.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{pred, App, HttpResponse};
///
/// fn main() {
///     App::new()
///         .resource("/index.html", |r| r.route()
///             .filter(pred::Any(pred::Get()).or(pred::Post()))
///             .f(|r| HttpResponse::MethodNotAllowed()));
/// }
/// ```
pub fn Any<S: 'static, P: Predicate<S> + 'static>(pred: P) -> AnyPredicate<S>
{
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
    fn check(&self, req: &mut HttpRequest<S>) -> bool {
        for p in &self.0 {
            if p.check(req) {
                return true
            }
        }
        false
    }
}

/// Return predicate that matches if all of supplied predicate matches.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{pred, Application, HttpResponse};
///
/// fn main() {
///     Application::new()
///         .resource("/index.html", |r| r.route()
///            .filter(pred::All(pred::Get())
///                 .and(pred::Header("content-type", "plain/text")))
///            .f(|_| HttpResponse::MethodNotAllowed()));
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
    fn check(&self, req: &mut HttpRequest<S>) -> bool {
        for p in &self.0 {
            if !p.check(req) {
                return false
            }
        }
        true
    }
}

/// Return predicate that matches if supplied predicate does not match.
pub fn Not<S: 'static, P: Predicate<S> + 'static>(pred: P) -> NotPredicate<S>
{
    NotPredicate(Box::new(pred))
}

#[doc(hidden)]
pub struct NotPredicate<S>(Box<Predicate<S>>);

impl<S: 'static> Predicate<S> for NotPredicate<S> {
    fn check(&self, req: &mut HttpRequest<S>) -> bool {
        !self.0.check(req)
    }
}

/// Http method predicate
#[doc(hidden)]
pub struct MethodPredicate<S>(http::Method, PhantomData<S>);

impl<S: 'static> Predicate<S> for MethodPredicate<S> {
    fn check(&self, req: &mut HttpRequest<S>) -> bool {
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

/// Return predicate that matches if request contains specified header and value.
pub fn Header<S: 'static>(name: &'static str, value: &'static str) -> HeaderPredicate<S>
{
    HeaderPredicate(header::HeaderName::try_from(name).unwrap(),
                    header::HeaderValue::from_static(value),
                    PhantomData)
}

#[doc(hidden)]
pub struct HeaderPredicate<S>(header::HeaderName, header::HeaderValue, PhantomData<S>);

impl<S: 'static> Predicate<S> for HeaderPredicate<S> {
    fn check(&self, req: &mut HttpRequest<S>) -> bool {
        if let Some(val) = req.headers().get(&self.0) {
            return val == self.1
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use http::{Uri, Version, Method};
    use http::header::{self, HeaderMap};

    #[test]
    fn test_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::TRANSFER_ENCODING,
                       header::HeaderValue::from_static("chunked"));
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, headers, None);

        let pred = Header("transfer-encoding", "chunked");
        assert!(pred.check(&mut req));

        let pred = Header("transfer-encoding", "other");
        assert!(!pred.check(&mut req));

        let pred = Header("content-type", "other");
        assert!(!pred.check(&mut req));
    }

    #[test]
    fn test_methods() {
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), None);
        let mut req2 = HttpRequest::new(
            Method::POST, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), None);

        assert!(Get().check(&mut req));
        assert!(!Get().check(&mut req2));
        assert!(Post().check(&mut req2));
        assert!(!Post().check(&mut req));

        let mut r = HttpRequest::new(
            Method::PUT, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), None);
        assert!(Put().check(&mut r));
        assert!(!Put().check(&mut req));

        let mut r = HttpRequest::new(
            Method::DELETE, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), None);
        assert!(Delete().check(&mut r));
        assert!(!Delete().check(&mut req));

        let mut r = HttpRequest::new(
            Method::HEAD, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), None);
        assert!(Head().check(&mut r));
        assert!(!Head().check(&mut req));

        let mut r = HttpRequest::new(
            Method::OPTIONS, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), None);
        assert!(Options().check(&mut r));
        assert!(!Options().check(&mut req));

        let mut r = HttpRequest::new(
            Method::CONNECT, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), None);
        assert!(Connect().check(&mut r));
        assert!(!Connect().check(&mut req));

        let mut r = HttpRequest::new(
            Method::PATCH, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), None);
        assert!(Patch().check(&mut r));
        assert!(!Patch().check(&mut req));

        let mut r = HttpRequest::new(
            Method::TRACE, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), None);
        assert!(Trace().check(&mut r));
        assert!(!Trace().check(&mut req));
    }

    #[test]
    fn test_preds() {
        let mut r = HttpRequest::new(
            Method::TRACE, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), None);

        assert!(Not(Get()).check(&mut r));
        assert!(!Not(Trace()).check(&mut r));

        assert!(All(Trace()).and(Trace()).check(&mut r));
        assert!(!All(Get()).and(Trace()).check(&mut r));

        assert!(Any(Get()).or(Trace()).check(&mut r));
        assert!(!Any(Get()).or(Get()).check(&mut r));
    }
}
