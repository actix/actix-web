use std::marker::PhantomData;

use regex::Regex;
use futures::future::{Future, ok, err};
use http::{header, StatusCode};

use body::Body;
use error::Error;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

/// Trait defines object that could be registered as route handler
#[allow(unused_variables)]
pub trait Handler<S>: 'static {

    /// The type of value that handler will return.
    type Result: Responder;

    /// Handle request
    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result;
}

/// Trait implemented by types that generate responses for clients.
///
/// Types that implement this trait can be used as the return type of a handler.
pub trait Responder {
    /// The associated item which can be returned.
    type Item: Into<Reply>;

    /// The associated error which can be returned.
    type Error: Into<Error>;

    /// Convert itself to `Reply` or `Error`.
    fn respond_to(self, req: HttpRequest) -> Result<Self::Item, Self::Error>;
}

/// Combines two different responder types into a single type
///
/// ```rust
/// # extern crate actix_web;
/// # extern crate futures;
/// # use futures::future::Future;
/// use actix_web::AsyncResponder;
/// use futures::future::result;
/// use actix_web::{Either, Error, HttpRequest, HttpResponse, httpcodes};
///
/// type RegisterResult = Either<HttpResponse, Box<Future<Item=HttpResponse, Error=Error>>>;
///
///
/// fn index(req: HttpRequest) -> RegisterResult {
///     if is_a_variant() { // <- choose variant A
///         Either::A(
///             httpcodes::HttpBadRequest.with_body("Bad data"))
///     } else {
///         Either::B(      // <- variant B
///             result(HttpResponse::Ok()
///                    .content_type("text/html")
///                    .body(format!("Hello!"))
///                    .map_err(|e| e.into())).responder())
///     }
/// }
/// # fn is_a_variant() -> bool { true }
/// # fn main() {}
/// ```
#[derive(Debug)]
pub enum Either<A, B> {
    /// First branch of the type
    A(A),
    /// Second branch of the type
    B(B),
}

impl<A, B> Responder for Either<A, B>
    where A: Responder, B: Responder
{
    type Item = Reply;
    type Error = Error;

    fn respond_to(self, req: HttpRequest) -> Result<Reply, Error> {
        match self {
            Either::A(a) => match a.respond_to(req) {
                Ok(val) => Ok(val.into()),
                Err(err) => Err(err.into()),
            },
            Either::B(b) => match b.respond_to(req) {
                Ok(val) => Ok(val.into()),
                Err(err) => Err(err.into()),
            },
        }
    }
}


#[doc(hidden)]
/// Convenience trait that convert `Future` object into `Boxed` future
pub trait AsyncResponder<I, E>: Sized {
    fn responder(self) -> Box<Future<Item=I, Error=E>>;
}

impl<F, I, E> AsyncResponder<I, E> for F
    where F: Future<Item=I, Error=E> + 'static,
          I: Responder + 'static,
          E: Into<Error> + 'static,
{
    fn responder(self) -> Box<Future<Item=I, Error=E>> {
        Box::new(self)
    }
}

/// Handler<S> for Fn()
impl<F, R, S> Handler<S> for F
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Responder + 'static
{
    type Result = R;

    fn handle(&mut self, req: HttpRequest<S>) -> R {
        (self)(req)
    }
}

/// Represents response process.
pub struct Reply(ReplyItem);

pub(crate) enum ReplyItem {
    Message(HttpResponse),
    Future(Box<Future<Item=HttpResponse, Error=Error>>),
}

impl Reply {

    /// Create async response
    #[inline]
    pub fn async<F>(fut: F) -> Reply
        where F: Future<Item=HttpResponse, Error=Error> + 'static
    {
        Reply(ReplyItem::Future(Box::new(fut)))
    }

    /// Send response
    #[inline]
    pub fn response<R: Into<HttpResponse>>(response: R) -> Reply {
        Reply(ReplyItem::Message(response.into()))
    }

    #[inline]
    pub(crate) fn into(self) -> ReplyItem {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn as_response(&self) -> Option<&HttpResponse> {
        match self.0 {
            ReplyItem::Message(ref resp) => Some(resp),
            _ => None,
        }
    }
}

impl Responder for Reply {
    type Item = Reply;
    type Error = Error;

    fn respond_to(self, _: HttpRequest) -> Result<Reply, Error> {
        Ok(self)
    }
}

impl Responder for HttpResponse {
    type Item = Reply;
    type Error = Error;

    #[inline]
    fn respond_to(self, _: HttpRequest) -> Result<Reply, Error> {
        Ok(Reply(ReplyItem::Message(self)))
    }
}

impl From<HttpResponse> for Reply {

    #[inline]
    fn from(resp: HttpResponse) -> Reply {
        Reply(ReplyItem::Message(resp))
    }
}

impl<T: Responder, E: Into<Error>> Responder for Result<T, E>
{
    type Item = <T as Responder>::Item;
    type Error = Error;

    fn respond_to(self, req: HttpRequest) -> Result<Self::Item, Self::Error> {
        match self {
            Ok(val) => match val.respond_to(req) {
                Ok(val) => Ok(val),
                Err(err) => Err(err.into()),
            },
            Err(err) => Err(err.into()),
        }
    }
}

impl<E: Into<Error>> From<Result<Reply, E>> for Reply {
    #[inline]
    fn from(res: Result<Reply, E>) -> Self {
        match res {
            Ok(val) => val,
            Err(err) => Reply(ReplyItem::Message(err.into().into())),
        }
    }
}

impl<E: Into<Error>> From<Result<HttpResponse, E>> for Reply {
    #[inline]
    fn from(res: Result<HttpResponse, E>) -> Self {
        match res {
            Ok(val) => Reply(ReplyItem::Message(val)),
            Err(err) => Reply(ReplyItem::Message(err.into().into())),
        }
    }
}

impl From<Box<Future<Item=HttpResponse, Error=Error>>> for Reply {
    #[inline]
    fn from(fut: Box<Future<Item=HttpResponse, Error=Error>>) -> Reply {
        Reply(ReplyItem::Future(fut))
    }
}

/// Convenience type alias
pub type FutureResponse<I, E=Error> = Box<Future<Item=I, Error=E>>;

impl<I, E> Responder for Box<Future<Item=I, Error=E>>
    where I: Responder + 'static,
          E: Into<Error> + 'static
{
    type Item = Reply;
    type Error = Error;

    #[inline]
    fn respond_to(self, req: HttpRequest) -> Result<Reply, Error> {
        let fut = self.map_err(|e| e.into())
            .then(move |r| {
                match r.respond_to(req) {
                    Ok(reply) => match reply.into().0 {
                        ReplyItem::Message(resp) => ok(resp),
                        _ => panic!("Nested async replies are not supported"),
                    },
                    Err(e) => err(e),
                }
            });
        Ok(Reply::async(fut))
    }
}

/// Trait defines object that could be registered as resource route
pub(crate) trait RouteHandler<S>: 'static {
    fn handle(&mut self, req: HttpRequest<S>) -> Reply;
}

/// Route handler wrapper for Handler
pub(crate)
struct WrapHandler<S, H, R>
    where H: Handler<S, Result=R>,
          R: Responder,
          S: 'static,
{
    h: H,
    s: PhantomData<S>,
}

impl<S, H, R> WrapHandler<S, H, R>
    where H: Handler<S, Result=R>,
          R: Responder,
          S: 'static,
{
    pub fn new(h: H) -> Self {
        WrapHandler{h, s: PhantomData}
    }
}

impl<S, H, R> RouteHandler<S> for WrapHandler<S, H, R>
    where H: Handler<S, Result=R>,
          R: Responder + 'static,
          S: 'static,
{
    fn handle(&mut self, req: HttpRequest<S>) -> Reply {
        let req2 = req.without_state();
        match self.h.handle(req).respond_to(req2) {
            Ok(reply) => reply.into(),
            Err(err) => Reply::response(err.into()),
        }
    }
}

/// Async route handler
pub(crate)
struct AsyncHandler<S, H, F, R, E>
    where H: Fn(HttpRequest<S>) -> F + 'static,
          F: Future<Item=R, Error=E> + 'static,
          R: Responder + 'static,
          E: Into<Error> + 'static,
          S: 'static,
{
    h: Box<H>,
    s: PhantomData<S>,
}

impl<S, H, F, R, E> AsyncHandler<S, H, F, R, E>
    where H: Fn(HttpRequest<S>) -> F + 'static,
          F: Future<Item=R, Error=E> + 'static,
          R: Responder + 'static,
          E: Into<Error> + 'static,
          S: 'static,
{
    pub fn new(h: H) -> Self {
        AsyncHandler{h: Box::new(h), s: PhantomData}
    }
}

impl<S, H, F, R, E> RouteHandler<S> for AsyncHandler<S, H, F, R, E>
    where H: Fn(HttpRequest<S>) -> F + 'static,
          F: Future<Item=R, Error=E> + 'static,
          R: Responder + 'static,
          E: Into<Error> + 'static,
          S: 'static,
{
    fn handle(&mut self, req: HttpRequest<S>) -> Reply {
        let req2 = req.without_state();
        let fut = (self.h)(req)
            .map_err(|e| e.into())
            .then(move |r| {
                match r.respond_to(req2) {
                    Ok(reply) => match reply.into().0 {
                        ReplyItem::Message(resp) => ok(resp),
                        _ => panic!("Nested async replies are not supported"),
                    },
                    Err(e) => err(e),
                }
            });
        Reply::async(fut)
    }
}

/// Path normalization helper
///
/// By normalizing it means:
///
/// - Add a trailing slash to the path.
/// - Remove a trailing slash from the path.
/// - Double slashes are replaced by one.
///
/// The handler returns as soon as it finds a path that resolves
/// correctly. The order if all enable is 1) merge, 3) both merge and append
/// and 3) append. If the path resolves with
/// at least one of those conditions, it will redirect to the new path.
///
/// If *append* is *true* append slash when needed. If a resource is
/// defined with trailing slash and the request comes without it, it will
/// append it automatically.
///
/// If *merge* is *true*, merge multiple consecutive slashes in the path into one.
///
/// This handler designed to be use as a handler for application's *default resource*.
///
/// ```rust
/// # extern crate actix_web;
/// # #[macro_use] extern crate serde_derive;
/// # use actix_web::*;
/// #
/// # fn index(req: HttpRequest) -> httpcodes::StaticResponse {
/// #     httpcodes::HttpOk
/// # }
/// fn main() {
///     let app = Application::new()
///         .resource("/test/", |r| r.f(index))
///         .default_resource(|r| r.h(NormalizePath::default()))
///         .finish();
/// }
/// ```
/// In this example `/test`, `/test///` will be redirected to `/test/` url.
pub struct NormalizePath {
    append: bool,
    merge: bool,
    re_merge: Regex,
    redirect: StatusCode,
    not_found: StatusCode,
}

impl Default for NormalizePath {
    /// Create default `NormalizePath` instance, *append* is set to *true*,
    /// *merge* is set to *true* and *redirect* is set to `StatusCode::MOVED_PERMANENTLY`
    fn default() -> NormalizePath {
        NormalizePath {
            append: true,
            merge: true,
            re_merge: Regex::new("//+").unwrap(),
            redirect: StatusCode::MOVED_PERMANENTLY,
            not_found: StatusCode::NOT_FOUND,
        }
    }
}

impl NormalizePath {
    /// Create new `NormalizePath` instance
    pub fn new(append: bool, merge: bool, redirect: StatusCode) -> NormalizePath {
        NormalizePath {
            append,
            merge,
            redirect,
            re_merge: Regex::new("//+").unwrap(),
            not_found: StatusCode::NOT_FOUND,
        }
    }
}

impl<S> Handler<S> for NormalizePath {
    type Result = Result<HttpResponse, Error>;

    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        if let Some(router) = req.router() {
            let query = req.query_string();
            if self.merge {
                // merge slashes
                let p = self.re_merge.replace_all(req.path(), "/");
                if p.len() != req.path().len() {
                    if router.has_route(p.as_ref()) {
                        let p = if !query.is_empty() { p + "?" + query } else { p };
                        return HttpResponse::build(self.redirect)
                            .header(header::LOCATION, p.as_ref())
                            .body(Body::Empty);
                    }
                    // merge slashes and append trailing slash
                    if self.append && !p.ends_with('/') {
                        let p = p.as_ref().to_owned() + "/";
                        if router.has_route(&p) {
                            let p = if !query.is_empty() { p + "?" + query } else { p };
                            return HttpResponse::build(self.redirect)
                                .header(header::LOCATION, p.as_str())
                                .body(Body::Empty);
                        }
                    }

                    // try to remove trailing slash
                    if p.ends_with('/') {
                        let p = p.as_ref().trim_right_matches('/');
                        if router.has_route(p) {
                            let mut req = HttpResponse::build(self.redirect);
                            return if !query.is_empty() {
                                req.header(header::LOCATION, (p.to_owned() + "?" + query).as_str())
                            } else {
                                req.header(header::LOCATION, p)
                            }
                            .body(Body::Empty);
                        }
                    }
                } else if p.ends_with('/') {
                    // try to remove trailing slash
                    let p = p.as_ref().trim_right_matches('/');
                    if router.has_route(p) {
                        let mut req = HttpResponse::build(self.redirect);
                        return if !query.is_empty() {
                            req.header(header::LOCATION, (p.to_owned() + "?" + query).as_str())
                        } else {
                            req.header(header::LOCATION, p)
                        }
                        .body(Body::Empty);
                    }
                }
            }
            // append trailing slash
            if self.append && !req.path().ends_with('/') {
                let p = req.path().to_owned() + "/";
                if router.has_route(&p) {
                    let p = if !query.is_empty() { p + "?" + query } else { p };
                    return HttpResponse::build(self.redirect)
                        .header(header::LOCATION, p.as_str())
                        .body(Body::Empty);
                }
            }
        }
        Ok(HttpResponse::new(self.not_found, Body::Empty))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{header, Method};
    use test::TestRequest;
    use application::Application;

    fn index(_req: HttpRequest) -> HttpResponse {
        HttpResponse::new(StatusCode::OK, Body::Empty)
    }

    #[test]
    fn test_normalize_path_trailing_slashes() {
        let mut app = Application::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource2/", |r| r.method(Method::GET).f(index))
            .default_resource(|r| r.h(NormalizePath::default()))
            .finish();

        // trailing slashes
        let params =
            vec![("/resource1", "", StatusCode::OK),
                 ("/resource1/", "/resource1", StatusCode::MOVED_PERMANENTLY),
                 ("/resource2", "/resource2/", StatusCode::MOVED_PERMANENTLY),
                 ("/resource2/", "", StatusCode::OK),
                 ("/resource1?p1=1&p2=2", "", StatusCode::OK),
                 ("/resource1/?p1=1&p2=2", "/resource1?p1=1&p2=2", StatusCode::MOVED_PERMANENTLY),
                 ("/resource2?p1=1&p2=2", "/resource2/?p1=1&p2=2",
                  StatusCode::MOVED_PERMANENTLY),
                 ("/resource2/?p1=1&p2=2", "", StatusCode::OK)
            ];
        for (path, target, code) in params {
            let req = app.prepare_request(TestRequest::with_uri(path).finish());
            let resp = app.run(req);
            let r = resp.as_response().unwrap();
            assert_eq!(r.status(), code);
            if !target.is_empty() {
                assert_eq!(
                    target,
                    r.headers().get(header::LOCATION).unwrap().to_str().unwrap());
            }
        }
    }

    #[test]
    fn test_normalize_path_trailing_slashes_disabled() {
        let mut app = Application::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource2/", |r| r.method(Method::GET).f(index))
            .default_resource(|r| r.h(
                NormalizePath::new(false, true, StatusCode::MOVED_PERMANENTLY)))
            .finish();

        // trailing slashes
        let params = vec![("/resource1", StatusCode::OK),
                          ("/resource1/", StatusCode::MOVED_PERMANENTLY),
                          ("/resource2", StatusCode::NOT_FOUND),
                          ("/resource2/", StatusCode::OK),
                          ("/resource1?p1=1&p2=2", StatusCode::OK),
                          ("/resource1/?p1=1&p2=2", StatusCode::MOVED_PERMANENTLY),
                          ("/resource2?p1=1&p2=2", StatusCode::NOT_FOUND),
                          ("/resource2/?p1=1&p2=2", StatusCode::OK)
        ];
        for (path, code) in params {
            let req = app.prepare_request(TestRequest::with_uri(path).finish());
            let resp = app.run(req);
            let r = resp.as_response().unwrap();
            assert_eq!(r.status(), code);
        }
    }

    #[test]
    fn test_normalize_path_merge_slashes() {
        let mut app = Application::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource1/a/b", |r| r.method(Method::GET).f(index))
            .default_resource(|r| r.h(NormalizePath::default()))
            .finish();

        // trailing slashes
        let params = vec![
            ("/resource1/a/b", "", StatusCode::OK),
            ("/resource1/", "/resource1", StatusCode::MOVED_PERMANENTLY),
            ("/resource1//", "/resource1", StatusCode::MOVED_PERMANENTLY),
            ("//resource1//a//b", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("//resource1//a//b/", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("//resource1//a//b//", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("///resource1//a//b", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a///b", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a//b/", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("/resource1/a/b?p=1", "", StatusCode::OK),
            ("//resource1//a//b?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("//resource1//a//b/?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("///resource1//a//b?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a///b?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a//b/?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a//b//?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
        ];
        for (path, target, code) in params {
            let req = app.prepare_request(TestRequest::with_uri(path).finish());
            let resp = app.run(req);
            let r = resp.as_response().unwrap();
            assert_eq!(r.status(), code);
            if !target.is_empty() {
                assert_eq!(
                    target,
                    r.headers().get(header::LOCATION).unwrap().to_str().unwrap());
            }
        }
    }

    #[test]
    fn test_normalize_path_merge_and_append_slashes() {
        let mut app = Application::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource2/", |r| r.method(Method::GET).f(index))
            .resource("/resource1/a/b", |r| r.method(Method::GET).f(index))
            .resource("/resource2/a/b/", |r| r.method(Method::GET).f(index))
            .default_resource(|r| r.h(NormalizePath::default()))
            .finish();

        // trailing slashes
        let params = vec![
            ("/resource1/a/b", "", StatusCode::OK),
            ("/resource1/a/b/", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b/", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b//", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("///resource1//a//b", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("///resource1//a//b/", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a///b", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a///b/", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("/resource2/a/b", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("/resource2/a/b/", "", StatusCode::OK),
            ("//resource2//a//b", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b/", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("///resource2//a//b", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("///resource2//a//b/", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("/////resource2/a///b", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("/////resource2/a///b/", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("/resource1/a/b?p=1", "", StatusCode::OK),
            ("/resource1/a/b/?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b/?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("///resource1//a//b?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("///resource1//a//b/?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a///b?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a///b/?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a///b//?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/resource2/a/b?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b/?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("///resource2//a//b?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("///resource2//a//b/?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource2/a///b?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource2/a///b/?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
        ];
        for (path, target, code) in params {
            let req = app.prepare_request(TestRequest::with_uri(path).finish());
            let resp = app.run(req);
            let r = resp.as_response().unwrap();
            assert_eq!(r.status(), code);
            if !target.is_empty() {
                assert_eq!(
                    target, r.headers().get(header::LOCATION).unwrap().to_str().unwrap());
            }
        }
    }


}
