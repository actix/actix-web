use std::mem;
use std::rc::Rc;
use std::marker::PhantomData;
use futures::{Async, Future, Poll};

use error::Error;
use pred::Predicate;
use http::StatusCode;
use handler::{Reply, ReplyItem, Handler, FromRequest,
              Responder, RouteHandler, AsyncHandler, WrapHandler};
use middleware::{Middleware, Response as MiddlewareResponse, Started as MiddlewareStarted};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use with::{With, With2, With3, ExtractorConfig};

/// Resource route definition
///
/// Route uses builder-like pattern for configuration.
/// If handler is not explicitly set, default *404 Not Found* handler is used.
pub struct Route<S> {
    preds: Vec<Box<Predicate<S>>>,
    handler: InnerHandler<S>,
}

impl<S: 'static> Default for Route<S> {

    fn default() -> Route<S> {
        Route {
            preds: Vec::new(),
            handler: InnerHandler::new(|_| HttpResponse::new(StatusCode::NOT_FOUND)),
        }
    }
}

impl<S: 'static> Route<S> {

    #[inline]
    pub(crate) fn check(&self, req: &mut HttpRequest<S>) -> bool {
        for pred in &self.preds {
            if !pred.check(req) {
                return false
            }
        }
        true
    }

    #[inline]
    pub(crate) fn handle(&mut self, req: HttpRequest<S>) -> Reply {
        self.handler.handle(req)
    }

    #[inline]
    pub(crate) fn compose(&mut self,
                          req: HttpRequest<S>,
                          mws: Rc<Vec<Box<Middleware<S>>>>) -> Reply {
        Reply::async(Compose::new(req, mws, self.handler.clone()))
    }

    /// Add match predicate to route.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # fn main() {
    /// App::new()
    ///    .resource("/path", |r|
    ///       r.route()
    ///          .filter(pred::Get())
    ///          .filter(pred::Header("content-type", "text/plain"))
    ///          .f(|req| HttpResponse::Ok())
    ///       )
    /// #      .finish();
    /// # }
    /// ```
    pub fn filter<T: Predicate<S> + 'static>(&mut self, p: T) -> &mut Self {
        self.preds.push(Box::new(p));
        self
    }

    /// Set handler object. Usually call to this method is last call
    /// during route configuration, so it does not return reference to self.
    pub fn h<H: Handler<S>>(&mut self, handler: H) {
        self.handler = InnerHandler::new(handler);
    }

    /// Set handler function. Usually call to this method is last call
    /// during route configuration, so it does not return reference to self.
    pub fn f<F, R>(&mut self, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: Responder + 'static,
    {
        self.handler = InnerHandler::new(handler);
    }

    /// Set async handler function.
    pub fn a<H, R, F, E>(&mut self, handler: H)
        where H: Fn(HttpRequest<S>) -> F + 'static,
              F: Future<Item=R, Error=E> + 'static,
              R: Responder + 'static,
              E: Into<Error> + 'static
    {
        self.handler = InnerHandler::async(handler);
    }

    /// Set handler function with http request extractor.
    ///
    /// ```rust
    /// # extern crate bytes;
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// #[macro_use] extern crate serde_derive;
    /// use actix_web::{App, Path, Result, http};
    ///
    /// #[derive(Deserialize)]
    /// struct Info {
    ///     username: String,
    /// }
    ///
    /// /// extract path info using serde
    /// fn index(info: Path<Info>) -> Result<String> {
    ///     Ok(format!("Welcome {}!", info.username))
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().resource(
    ///        "/{username}/index.html",                     // <- define path parameters
    ///        |r| r.method(http::Method::GET).with(index)); // <- use `with` extractor
    /// }
    /// ```
    pub fn with<T, F, R>(&mut self, handler: F) -> ExtractorConfig<S, T>
        where F: Fn(T) -> R + 'static,
              R: Responder + 'static,
              T: FromRequest<S> + 'static,
    {
        let cfg = ExtractorConfig::default();
        self.h(With::new(handler, Clone::clone(&cfg)));
        cfg
    }

    /// Set handler function, function has to accept two request extractors.
    ///
    /// ```rust
    /// # extern crate bytes;
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// #[macro_use] extern crate serde_derive;
    /// use actix_web::{App, Query, Path, Result, http};
    ///
    /// #[derive(Deserialize)]
    /// struct PParam {
    ///     username: String,
    /// }
    ///
    /// #[derive(Deserialize)]
    /// struct QParam {
    ///     count: u32,
    /// }
    ///
    /// /// extract path and query information using serde
    /// fn index(p: Path<PParam>, q: Query<QParam>) -> Result<String> {
    ///     Ok(format!("Welcome {}!", p.username))
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().resource(
    ///        "/{username}/index.html",                      // <- define path parameters
    ///        |r| r.method(http::Method::GET).with2(index)); // <- use `with` extractor
    /// }
    /// ```
    pub fn with2<T1, T2, F, R>(&mut self, handler: F)
                               -> (ExtractorConfig<S, T1>, ExtractorConfig<S, T2>)
        where F: Fn(T1, T2) -> R + 'static,
              R: Responder + 'static,
              T1: FromRequest<S> + 'static,
              T2: FromRequest<S> + 'static,
    {
        let cfg1 = ExtractorConfig::default();
        let cfg2 = ExtractorConfig::default();
        self.h(With2::new(handler, Clone::clone(&cfg1), Clone::clone(&cfg2)));
        (cfg1, cfg2)
    }

    /// Set handler function, function has to accept three request extractors.
    pub fn with3<T1, T2, T3, F, R>(&mut self, handler: F)
        -> (ExtractorConfig<S, T1>, ExtractorConfig<S, T2>, ExtractorConfig<S, T3>)
        where F: Fn(T1, T2, T3) -> R + 'static,
              R: Responder + 'static,
              T1: FromRequest<S> + 'static,
              T2: FromRequest<S> + 'static,
              T3: FromRequest<S> + 'static,
    {
        let cfg1 = ExtractorConfig::default();
        let cfg2 = ExtractorConfig::default();
        let cfg3 = ExtractorConfig::default();
        self.h(With3::new(
            handler, Clone::clone(&cfg1), Clone::clone(&cfg2), Clone::clone(&cfg3)));
        (cfg1, cfg2, cfg3)
    }
}

/// `RouteHandler` wrapper. This struct is required because it needs to be shared
/// for resource level middlewares.
struct InnerHandler<S>(Rc<Box<RouteHandler<S>>>);

impl<S: 'static> InnerHandler<S> {

    #[inline]
    fn new<H: Handler<S>>(h: H) -> Self {
        InnerHandler(Rc::new(Box::new(WrapHandler::new(h))))
    }

    #[inline]
    fn async<H, R, F, E>(h: H) -> Self
        where H: Fn(HttpRequest<S>) -> F + 'static,
              F: Future<Item=R, Error=E> + 'static,
              R: Responder + 'static,
              E: Into<Error> + 'static
    {
        InnerHandler(Rc::new(Box::new(AsyncHandler::new(h))))
    }

    #[inline]
    pub fn handle(&self, req: HttpRequest<S>) -> Reply {
        // reason: handler is unique per thread,
        // handler get called from async code only
        #[allow(mutable_transmutes)]
        #[cfg_attr(feature = "cargo-clippy", allow(borrowed_box))]
        let h: &mut Box<RouteHandler<S>> = unsafe { mem::transmute(self.0.as_ref()) };
        h.handle(req)
    }
}

impl<S> Clone for InnerHandler<S> {
    #[inline]
    fn clone(&self) -> Self {
        InnerHandler(Rc::clone(&self.0))
    }
}


/// Compose resource level middlewares with route handler.
struct Compose<S: 'static> {
    info: ComposeInfo<S>,
    state: ComposeState<S>,
}

struct ComposeInfo<S: 'static> {
    count: usize,
    req: HttpRequest<S>,
    mws: Rc<Vec<Box<Middleware<S>>>>,
    handler: InnerHandler<S>,
}

enum ComposeState<S: 'static> {
    Starting(StartMiddlewares<S>),
    Handler(WaitingResponse<S>),
    RunMiddlewares(RunMiddlewares<S>),
    Response(Response<S>),
}

impl<S: 'static> ComposeState<S> {
    fn poll(&mut self, info: &mut ComposeInfo<S>) -> Option<ComposeState<S>> {
        match *self {
            ComposeState::Starting(ref mut state) => state.poll(info),
            ComposeState::Handler(ref mut state) => state.poll(info),
            ComposeState::RunMiddlewares(ref mut state) => state.poll(info),
            ComposeState::Response(_) => None,
        }
    }
}

impl<S: 'static> Compose<S> {
    fn new(req: HttpRequest<S>,
           mws: Rc<Vec<Box<Middleware<S>>>>,
           handler: InnerHandler<S>) -> Self
    {
        let mut info = ComposeInfo { count: 0, req, mws, handler };
        let state = StartMiddlewares::init(&mut info);

        Compose {state, info}
    }
}

impl<S> Future for Compose<S> {
    type Item = HttpResponse;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            if let ComposeState::Response(ref mut resp) = self.state {
                let resp = resp.resp.take().unwrap();
                return Ok(Async::Ready(resp))
            }
            if let Some(state) = self.state.poll(&mut self.info) {
                self.state = state;
            } else {
                return Ok(Async::NotReady)
            }
        }
    }
}

/// Middlewares start executor
struct StartMiddlewares<S> {
    fut: Option<Fut>,
    _s: PhantomData<S>,
}

type Fut = Box<Future<Item=Option<HttpResponse>, Error=Error>>;

impl<S: 'static> StartMiddlewares<S> {

    fn init(info: &mut ComposeInfo<S>) -> ComposeState<S> {
        let len = info.mws.len();
        loop {
            if info.count == len {
                let reply = info.handler.handle(info.req.clone());
                return WaitingResponse::init(info, reply)
            } else {
                match info.mws[info.count].start(&mut info.req) {
                    Ok(MiddlewareStarted::Done) =>
                        info.count += 1,
                    Ok(MiddlewareStarted::Response(resp)) =>
                        return RunMiddlewares::init(info, resp),
                    Ok(MiddlewareStarted::Future(mut fut)) =>
                        match fut.poll() {
                            Ok(Async::NotReady) =>
                                return ComposeState::Starting(StartMiddlewares {
                                    fut: Some(fut),
                                    _s: PhantomData}),
                            Ok(Async::Ready(resp)) => {
                                if let Some(resp) = resp {
                                    return RunMiddlewares::init(info, resp);
                                }
                                info.count += 1;
                            }
                            Err(err) =>
                                return Response::init(err.into()),
                        },
                    Err(err) =>
                        return Response::init(err.into()),
                }
            }
        }
    }

    fn poll(&mut self, info: &mut ComposeInfo<S>) -> Option<ComposeState<S>>
    {
        let len = info.mws.len();
        'outer: loop {
            match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) =>
                    return None,
                Ok(Async::Ready(resp)) => {
                    info.count += 1;
                    if let Some(resp) = resp {
                        return Some(RunMiddlewares::init(info, resp));
                    }
                    if info.count == len {
                        let reply = info.handler.handle(info.req.clone());
                        return Some(WaitingResponse::init(info, reply));
                    } else {
                        loop {
                            match info.mws[info.count].start(&mut info.req) {
                                Ok(MiddlewareStarted::Done) =>
                                    info.count += 1,
                                Ok(MiddlewareStarted::Response(resp)) => {
                                    return Some(RunMiddlewares::init(info, resp));
                                },
                                Ok(MiddlewareStarted::Future(fut)) => {
                                    self.fut = Some(fut);
                                    continue 'outer
                                },
                                Err(err) =>
                                    return Some(Response::init(err.into()))
                            }
                        }
                    }
                }
                Err(err) =>
                    return Some(Response::init(err.into()))
            }
        }
    }
}

// waiting for response
struct WaitingResponse<S> {
    fut: Box<Future<Item=HttpResponse, Error=Error>>,
    _s: PhantomData<S>,
}

impl<S: 'static> WaitingResponse<S> {

    #[inline]
    fn init(info: &mut ComposeInfo<S>, reply: Reply) -> ComposeState<S> {
        match reply.into() {
            ReplyItem::Message(resp) =>
                RunMiddlewares::init(info, resp),
            ReplyItem::Future(fut) =>
                ComposeState::Handler(
                    WaitingResponse { fut, _s: PhantomData }),
        }
    }

    fn poll(&mut self, info: &mut ComposeInfo<S>) -> Option<ComposeState<S>> {
        match self.fut.poll() {
            Ok(Async::NotReady) => None,
            Ok(Async::Ready(response)) =>
                Some(RunMiddlewares::init(info, response)),
            Err(err) =>
                Some(Response::init(err.into())),
        }
    }
}


/// Middlewares response executor
struct RunMiddlewares<S> {
    curr: usize,
    fut: Option<Box<Future<Item=HttpResponse, Error=Error>>>,
    _s: PhantomData<S>,
}

impl<S: 'static> RunMiddlewares<S> {

    fn init(info: &mut ComposeInfo<S>, mut resp: HttpResponse) -> ComposeState<S> {
        let mut curr = 0;
        let len = info.mws.len();

        loop {
            resp = match info.mws[curr].response(&mut info.req, resp) {
                Err(err) => {
                    info.count = curr + 1;
                    return Response::init(err.into())
                },
                Ok(MiddlewareResponse::Done(r)) => {
                    curr += 1;
                    if curr == len {
                        return Response::init(r)
                    } else {
                        r
                    }
                },
                Ok(MiddlewareResponse::Future(fut)) => {
                    return ComposeState::RunMiddlewares(
                        RunMiddlewares { curr, fut: Some(fut), _s: PhantomData })
                },
            };
        }
    }
    
    fn poll(&mut self, info: &mut ComposeInfo<S>) -> Option<ComposeState<S>>
    {
        let len = info.mws.len();

        loop {
            // poll latest fut
            let mut resp = match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) => {
                    return None
                }
                Ok(Async::Ready(resp)) => {
                    self.curr += 1;
                    resp
                }
                Err(err) =>
                    return Some(Response::init(err.into())),
            };

            loop {
                if self.curr == len {
                    return Some(Response::init(resp));
                } else {
                    match info.mws[self.curr].response(&mut info.req, resp) {
                        Err(err) =>
                            return Some(Response::init(err.into())),
                        Ok(MiddlewareResponse::Done(r)) => {
                            self.curr += 1;
                            resp = r
                        },
                        Ok(MiddlewareResponse::Future(fut)) => {
                            self.fut = Some(fut);
                            break
                        },
                    }
                }
            }
        }
    }
}

struct Response<S> {
    resp: Option<HttpResponse>,
    _s: PhantomData<S>,
}

impl<S: 'static> Response<S> {

    fn init(resp: HttpResponse) -> ComposeState<S> {
        ComposeState::Response(
            Response{resp: Some(resp), _s: PhantomData})
    }
}
