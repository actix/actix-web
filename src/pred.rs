//! Route match predicates
#![allow(non_snake_case)]
use std::marker::PhantomData;
use http;
use http::{header, HttpTryFrom};
use httprequest::HttpRequest;

/// Trait defines resource route predicate.
/// Predicate can modify request object. It is also possible to
/// to store extra attributes on request by using `.extensions()` method.
pub trait Predicate<S> {

    /// Check if request matches predicate
    fn check(&self, &mut HttpRequest<S>) -> bool;

}

/// Return predicate that matches if any of supplied predicate matches.
pub fn Any<T, S: 'static>(preds: T) -> Box<Predicate<S>>
    where T: IntoIterator<Item=Box<Predicate<S>>>
{
    Box::new(AnyPredicate(preds.into_iter().collect()))
}

struct AnyPredicate<S>(Vec<Box<Predicate<S>>>);

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
pub fn All<T, S: 'static>(preds: T) -> Box<Predicate<S>>
    where T: IntoIterator<Item=Box<Predicate<S>>>
{
    Box::new(AllPredicate(preds.into_iter().collect()))
}

struct AllPredicate<S>(Vec<Box<Predicate<S>>>);

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
pub fn Not<S: 'static>(pred: Box<Predicate<S>>) -> Box<Predicate<S>>
{
    Box::new(NotPredicate(pred))
}

struct NotPredicate<S>(Box<Predicate<S>>);

impl<S: 'static> Predicate<S> for NotPredicate<S> {
    fn check(&self, req: &mut HttpRequest<S>) -> bool {
        !self.0.check(req)
    }
}

/// Http method predicate
struct MethodPredicate<S>(http::Method, PhantomData<S>);

impl<S: 'static> Predicate<S> for MethodPredicate<S> {
    fn check(&self, req: &mut HttpRequest<S>) -> bool {
        *req.method() == self.0
    }
}

/// Predicate to match *GET* http method
pub fn Get<S: 'static>() -> Box<Predicate<S>> {
    Box::new(MethodPredicate(http::Method::GET, PhantomData))
}

/// Predicate to match *POST* http method
pub fn Post<S: 'static>() -> Box<Predicate<S>> {
    Box::new(MethodPredicate(http::Method::POST, PhantomData))
}

/// Predicate to match *PUT* http method
pub fn Put<S: 'static>() -> Box<Predicate<S>> {
    Box::new(MethodPredicate(http::Method::PUT, PhantomData))
}

/// Predicate to match *DELETE* http method
pub fn Delete<S: 'static>() -> Box<Predicate<S>> {
    Box::new(MethodPredicate(http::Method::DELETE, PhantomData))
}

/// Predicate to match *HEAD* http method
pub fn Head<S: 'static>() -> Box<Predicate<S>> {
    Box::new(MethodPredicate(http::Method::HEAD, PhantomData))
}

/// Predicate to match *OPTIONS* http method
pub fn Options<S: 'static>() -> Box<Predicate<S>> {
    Box::new(MethodPredicate(http::Method::OPTIONS, PhantomData))
}

/// Predicate to match *CONNECT* http method
pub fn Connect<S: 'static>() -> Box<Predicate<S>> {
    Box::new(MethodPredicate(http::Method::CONNECT, PhantomData))
}

/// Predicate to match *PATCH* http method
pub fn Patch<S: 'static>() -> Box<Predicate<S>> {
    Box::new(MethodPredicate(http::Method::PATCH, PhantomData))
}

/// Predicate to match *TRACE* http method
pub fn Trace<S: 'static>() -> Box<Predicate<S>> {
    Box::new(MethodPredicate(http::Method::TRACE, PhantomData))
}

/// Predicate to match specified http method
pub fn Method<S: 'static>(method: http::Method) -> Box<Predicate<S>> {
    Box::new(MethodPredicate(method, PhantomData))
}

/// Return predicate that matches if request contains specified header and value.
pub fn Header<S: 'static>(name: &'static str, value: &'static str) -> Box<Predicate<S>>
{
    Box::new(HeaderPredicate(header::HeaderName::try_from(name).unwrap(),
                             header::HeaderValue::from_static(value),
                             PhantomData))
}

struct HeaderPredicate<S>(header::HeaderName, header::HeaderValue, PhantomData<S>);

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
    use payload::Payload;

    #[test]
    fn test_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::TRANSFER_ENCODING,
                       header::HeaderValue::from_static("chunked"));
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(),
            Version::HTTP_11, headers, Payload::empty());

        let pred = Header("transfer-encoding", "chunked");
        assert!(pred.check(&mut req));

        let pred = Header("transfer-encoding", "other");
        assert!(!pred.check(&mut req));

        let pred = Header("content-tye", "other");
        assert!(!pred.check(&mut req));
    }

    #[test]
    fn test_methods() {
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        let mut req2 = HttpRequest::new(
            Method::POST, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());

        assert!(Get().check(&mut req));
        assert!(!Get().check(&mut req2));
        assert!(Post().check(&mut req2));
        assert!(!Post().check(&mut req));

        let mut r = HttpRequest::new(
            Method::PUT, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(Put().check(&mut r));
        assert!(!Put().check(&mut req));

        let mut r = HttpRequest::new(
            Method::DELETE, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(Delete().check(&mut r));
        assert!(!Delete().check(&mut req));

        let mut r = HttpRequest::new(
            Method::HEAD, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(Head().check(&mut r));
        assert!(!Head().check(&mut req));

        let mut r = HttpRequest::new(
            Method::OPTIONS, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(Options().check(&mut r));
        assert!(!Options().check(&mut req));

        let mut r = HttpRequest::new(
            Method::CONNECT, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(Connect().check(&mut r));
        assert!(!Connect().check(&mut req));

        let mut r = HttpRequest::new(
            Method::PATCH, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(Patch().check(&mut r));
        assert!(!Patch().check(&mut req));

        let mut r = HttpRequest::new(
            Method::TRACE, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(Trace().check(&mut r));
        assert!(!Trace().check(&mut req));
    }

    #[test]
    fn test_preds() {
        let mut r = HttpRequest::new(
            Method::TRACE, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());

        assert!(Not(Get()).check(&mut r));
        assert!(!Not(Trace()).check(&mut r));

        assert!(All(vec![Trace(), Trace()]).check(&mut r));
        assert!(!All(vec![Get(), Trace()]).check(&mut r));

        assert!(Any(vec![Get(), Trace()]).check(&mut r));
        assert!(!Any(vec![Get(), Get()]).check(&mut r));
    }
}
