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
