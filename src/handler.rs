use std::marker::PhantomData;

use actix::Actor;
use futures::future::{Future, ok, err};
use serde_json;
use serde::Serialize;
use regex::Regex;
use http::{header, StatusCode, Error as HttpError};

use body::Body;
use error::Error;
use context::{HttpContext, IoContext};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

/// Trait defines object that could be regestered as route handler
#[allow(unused_variables)]
pub trait Handler<S>: 'static {

    /// The type of value that handler will return.
    type Result: Responder;

    /// Handle request
    fn handle(&self, req: HttpRequest<S>) -> Self::Result;
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

/// Handler<S> for Fn()
impl<F, R, S> Handler<S> for F
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Responder + 'static
{
    type Result = R;

    fn handle(&self, req: HttpRequest<S>) -> R {
        (self)(req)
    }
}

/// Represents response process.
pub struct Reply(ReplyItem);

pub(crate) enum ReplyItem {
    Message(HttpResponse),
    Actor(Box<IoContext>),
    Future(Box<Future<Item=HttpResponse, Error=Error>>),
}

impl Reply {

    /// Create actor response
    #[inline]
    pub fn actor<A, S>(ctx: HttpContext<A, S>) -> Reply
        where A: Actor<Context=HttpContext<A, S>>, S: 'static
    {
        Reply(ReplyItem::Actor(Box::new(ctx)))
    }

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
    fn from(res: Result<Reply, E>) -> Self {
        match res {
            Ok(val) => val,
            Err(err) => Reply(ReplyItem::Message(err.into().into())),
        }
    }
}

impl<A: Actor<Context=HttpContext<A, S>>, S: 'static> Responder for HttpContext<A, S>
{
    type Item = Reply;
    type Error = Error;

    #[inline]
    fn respond_to(self, _: HttpRequest) -> Result<Reply, Error> {
        Ok(Reply(ReplyItem::Actor(Box::new(self))))
    }
}

impl<A: Actor<Context=HttpContext<A, S>>, S: 'static> From<HttpContext<A, S>> for Reply {

    #[inline]
    fn from(ctx: HttpContext<A, S>) -> Reply {
        Reply(ReplyItem::Actor(Box::new(ctx)))
    }
}

impl Responder for Box<Future<Item=HttpResponse, Error=Error>>
{
    type Item = Reply;
    type Error = Error;

    #[inline]
    fn respond_to(self, _: HttpRequest) -> Result<Reply, Error> {
        Ok(Reply(ReplyItem::Future(self)))
    }
}

/// Trait defines object that could be regestered as resource route
pub(crate) trait RouteHandler<S>: 'static {
    fn handle(&self, req: HttpRequest<S>) -> Reply;
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
        WrapHandler{h: h, s: PhantomData}
    }
}

impl<S, H, R> RouteHandler<S> for WrapHandler<S, H, R>
    where H: Handler<S, Result=R>,
          R: Responder + 'static,
          S: 'static,
{
    fn handle(&self, req: HttpRequest<S>) -> Reply {
        let req2 = req.clone_without_state();
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
    fn handle(&self, req: HttpRequest<S>) -> Reply {
        let req2 = req.clone_without_state();
        let fut = (self.h)(req)
            .map_err(|e| e.into())
            .then(move |r| {
                match r.respond_to(req2) {
                    Ok(reply) => match reply.into().0 {
                        ReplyItem::Message(resp) => ok(resp),
                        _ => panic!("Nested async replies are not supported"),
                    }
                    Err(e) => err(e),
                }
            });
        Reply::async(fut)
    }
}

/// Json response helper
///
/// The `Json` type allows you to respond with well-formed JSON data: simply return a value of
/// type Json<T> where T is the type of a structure to serialize into *JSON*. The
/// type `T` must implement the `Serialize` trait from *serde*.
///
/// ```rust
/// # extern crate actix_web;
/// # #[macro_use] extern crate serde_derive;
/// # use actix_web::*;
/// # 
/// #[derive(Serialize)]
/// struct MyObj {
///     name: String,
/// }
///
/// fn index(req: HttpRequest) -> Result<Json<MyObj>> {
///     Ok(Json(MyObj{name: req.match_info().query("name")?}))
/// }
/// # fn main() {}
/// ```
pub struct Json<T: Serialize> (pub T);

impl<T: Serialize> Responder for Json<T> {
    type Item = HttpResponse;
    type Error = Error;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, Error> {
        let body = serde_json::to_string(&self.0)?;

        Ok(HttpResponse::Ok()
           .content_type("application/json")
           .body(body)?)
    }
}

/// Path normalization helper
///
/// By normalizing it means:
///
/// - Add a trailing slash to the path.
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
/// #     httpcodes::HTTPOk
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
    /// Create new `NoramlizePath` instance
    pub fn new(append: bool, merge: bool, redirect: StatusCode) -> NormalizePath {
        NormalizePath {
            append: append,
            merge: merge,
            re_merge: Regex::new("//+").unwrap(),
            redirect: redirect,
            not_found: StatusCode::NOT_FOUND,
        }
    }
}

impl<S> Handler<S> for NormalizePath {
    type Result = Result<HttpResponse, HttpError>;

    fn handle(&self, req: HttpRequest<S>) -> Self::Result {
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
    use application::Application;

    #[derive(Serialize)]
    struct MyObj {
        name: &'static str,
    }

    #[test]
    fn test_json() {
        let json = Json(MyObj{name: "test"});
        let resp = json.respond_to(HttpRequest::default()).unwrap();
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), "application/json");
    }

    fn index(_req: HttpRequest) -> HttpResponse {
        HttpResponse::new(StatusCode::OK, Body::Empty)
    }

    #[test]
    fn test_normalize_path_trailing_slashes() {
        let app = Application::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource2/", |r| r.method(Method::GET).f(index))
            .default_resource(|r| r.h(NormalizePath::default()))
            .finish();

        // trailing slashes
        let params = vec![("/resource1", "", StatusCode::OK),
                          ("/resource1/", "", StatusCode::NOT_FOUND),
                          ("/resource2", "/resource2/", StatusCode::MOVED_PERMANENTLY),
                          ("/resource2/", "", StatusCode::OK),
                          ("/resource1?p1=1&p2=2", "", StatusCode::OK),
                          ("/resource1/?p1=1&p2=2", "", StatusCode::NOT_FOUND),
                          ("/resource2?p1=1&p2=2", "/resource2/?p1=1&p2=2",
                           StatusCode::MOVED_PERMANENTLY),
                          ("/resource2/?p1=1&p2=2", "", StatusCode::OK)
        ];
        for (path, target, code) in params {
            let req = app.prepare_request(HttpRequest::from_path(path));
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
        let app = Application::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource2/", |r| r.method(Method::GET).f(index))
            .default_resource(|r| r.h(
                NormalizePath::new(false, true, StatusCode::MOVED_PERMANENTLY)))
            .finish();

        // trailing slashes
        let params = vec![("/resource1", StatusCode::OK),
                          ("/resource1/", StatusCode::NOT_FOUND),
                          ("/resource2", StatusCode::NOT_FOUND),
                          ("/resource2/", StatusCode::OK),
                          ("/resource1?p1=1&p2=2", StatusCode::OK),
                          ("/resource1/?p1=1&p2=2", StatusCode::NOT_FOUND),
                          ("/resource2?p1=1&p2=2", StatusCode::NOT_FOUND),
                          ("/resource2/?p1=1&p2=2", StatusCode::OK)
        ];
        for (path, code) in params {
            let req = app.prepare_request(HttpRequest::from_path(path));
            let resp = app.run(req);
            let r = resp.as_response().unwrap();
            assert_eq!(r.status(), code);
        }
    }

    #[test]
    fn test_normalize_path_merge_slashes() {
        let app = Application::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource1/a/b", |r| r.method(Method::GET).f(index))
            .default_resource(|r| r.h(NormalizePath::default()))
            .finish();

        // trailing slashes
        let params = vec![
            ("/resource1/a/b", "", StatusCode::OK),
            ("//resource1//a//b", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("//resource1//a//b/", "", StatusCode::NOT_FOUND),
            ("///resource1//a//b", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a///b", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a//b/", "", StatusCode::NOT_FOUND),
            ("/resource1/a/b?p=1", "", StatusCode::OK),
            ("//resource1//a//b?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("//resource1//a//b/?p=1", "", StatusCode::NOT_FOUND),
            ("///resource1//a//b?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a///b?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a//b/?p=1", "", StatusCode::NOT_FOUND),
        ];
        for (path, target, code) in params {
            let req = app.prepare_request(HttpRequest::from_path(path));
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
        let app = Application::new()
            .resource("/resource1", |r| r.method(Method::GET).f(index))
            .resource("/resource2/", |r| r.method(Method::GET).f(index))
            .resource("/resource1/a/b", |r| r.method(Method::GET).f(index))
            .resource("/resource2/a/b/", |r| r.method(Method::GET).f(index))
            .default_resource(|r| r.h(NormalizePath::default()))
            .finish();

        // trailing slashes
        let params = vec![
            ("/resource1/a/b", "", StatusCode::OK),
            ("/resource1/a/b/", "", StatusCode::NOT_FOUND),
            ("//resource2//a//b", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b/", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("///resource1//a//b", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("///resource1//a//b/", "", StatusCode::NOT_FOUND),
            ("/////resource1/a///b", "/resource1/a/b", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a///b/", "", StatusCode::NOT_FOUND),
            ("/resource2/a/b", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("/resource2/a/b/", "", StatusCode::OK),
            ("//resource2//a//b", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b/", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("///resource2//a//b", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("///resource2//a//b/", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("/////resource2/a///b", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("/////resource2/a///b/", "/resource2/a/b/", StatusCode::MOVED_PERMANENTLY),
            ("/resource1/a/b?p=1", "", StatusCode::OK),
            ("/resource1/a/b/?p=1", "", StatusCode::NOT_FOUND),
            ("//resource2//a//b?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b/?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("///resource1//a//b?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("///resource1//a//b/?p=1", "", StatusCode::NOT_FOUND),
            ("/////resource1/a///b?p=1", "/resource1/a/b?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource1/a///b/?p=1", "", StatusCode::NOT_FOUND),
            ("/resource2/a/b?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("//resource2//a//b/?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("///resource2//a//b?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("///resource2//a//b/?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource2/a///b?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
            ("/////resource2/a///b/?p=1", "/resource2/a/b/?p=1", StatusCode::MOVED_PERMANENTLY),
        ];
        for (path, target, code) in params {
            let req = app.prepare_request(HttpRequest::from_path(path));
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
