use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::rc::Rc;

use futures::{Async, Future, Poll};

use error::Error;
use handler::{FromRequest, Reply, ReplyItem, Responder, RouteHandler};
use http::Method;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::{Finished as MiddlewareFinished, Middleware,
                 Response as MiddlewareResponse, Started as MiddlewareStarted};
use resource::ResourceHandler;
use router::Resource;

type ScopeResources<S> = Rc<Vec<(Resource, Rc<UnsafeCell<ResourceHandler<S>>>)>>;

/// Resources scope
///
/// Scope is a set of resources with common root path.
/// Scopes collect multiple paths under a common path prefix.
/// Scope path can not contain variable path segments as resources.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{http, App, HttpRequest, HttpResponse};
///
/// fn main() {
///     let app = App::new()
///         .scope("/app", |scope| {
///              scope.resource("/path1", |r| r.f(|_| HttpResponse::Ok()))
///                .resource("/path2", |r| r.f(|_| HttpResponse::Ok()))
///                .resource("/path3", |r| r.f(|_| HttpResponse::MethodNotAllowed()))
///         });
/// }
/// ```
///
/// In the above example three routes get registered:
///  * /app/path1 - reponds to all http method
///  * /app/path2 - `GET` requests
///  * /app/path3 - `HEAD` requests
///
pub struct Scope<S: 'static> {
    handler: Option<UnsafeCell<Box<RouteHandler<S>>>>,
    middlewares: Rc<Vec<Box<Middleware<S>>>>,
    default: Rc<UnsafeCell<ResourceHandler<S>>>,
    resources: ScopeResources<S>,
}

impl<S: 'static> Default for Scope<S> {
    fn default() -> Scope<S> {
        Scope::new()
    }
}

impl<S: 'static> Scope<S> {
    pub fn new() -> Scope<S> {
        Scope {
            handler: None,
            resources: Rc::new(Vec::new()),
            middlewares: Rc::new(Vec::new()),
            default: Rc::new(UnsafeCell::new(ResourceHandler::default_not_found())),
        }
    }

    /// Create scope with new state.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, App, HttpRequest, HttpResponse, Path};
    ///
    /// struct AppState;
    ///
    /// fn index(req: HttpRequest<AppState>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .scope("/app", |scope| {
    ///             scope.with_state(AppState, |scope| {
    ///                scope.resource("/test1", |r| r.f(index))
    ///             })
    ///         });
    /// }
    /// ```
    pub fn with_state<F, T: 'static>(mut self, state: T, f: F) -> Scope<S>
    where
        F: FnOnce(Scope<T>) -> Scope<T>,
    {
        let scope = Scope {
            handler: None,
            resources: Rc::new(Vec::new()),
            middlewares: Rc::new(Vec::new()),
            default: Rc::new(UnsafeCell::new(ResourceHandler::default_not_found())),
        };
        let scope = f(scope);

        self.handler = Some(UnsafeCell::new(Box::new(Wrapper {
            scope,
            state: Rc::new(state),
        })));

        self
    }

    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `Scope::resource()` method.
    /// Handler functions need to accept one request extractor
    /// argument.
    ///
    /// This method could be called multiple times, in that case
    /// multiple routes would be registered for same resource path.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, App, HttpRequest, HttpResponse, Path};
    ///
    /// fn index(data: Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .scope("/app", |scope| {
    ///             scope.route("/test1", http::Method::GET, index)
    ///                .route("/test2", http::Method::POST,
    ///                    |_: HttpRequest| HttpResponse::MethodNotAllowed())
    ///         });
    /// }
    /// ```
    pub fn route<T, F, R>(mut self, path: &str, method: Method, f: F) -> Scope<S>
    where
        F: Fn(T) -> R + 'static,
        R: Responder + 'static,
        T: FromRequest<S> + 'static,
    {
        // get resource handler
        let slf: &Scope<S> = unsafe { &*(&self as *const _) };
        for &(ref pattern, ref resource) in slf.resources.iter() {
            if pattern.pattern() == path {
                let resource = unsafe { &mut *resource.get() };
                resource.method(method).with(f);
                return self;
            }
        }

        let mut handler = ResourceHandler::default();
        handler.method(method).with(f);
        let pattern = Resource::new(handler.get_name(), path);
        Rc::get_mut(&mut self.resources)
            .expect("Can not use after configuration")
            .push((pattern, Rc::new(UnsafeCell::new(handler))));

        self
    }

    /// Configure resource for a specific path.
    ///
    /// This method is similar to an `App::resource()` method.
    /// Resources may have variable path segments. Resource path uses scope
    /// path as a path prefix.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .scope("/api", |scope| {
    ///             scope.resource("/users/{userid}/{friend}", |r| {
    ///                 r.get().f(|_| HttpResponse::Ok());
    ///                 r.head().f(|_| HttpResponse::MethodNotAllowed());
    ///                 r.route()
    ///                    .filter(pred::Any(pred::Get()).or(pred::Put()))
    ///                    .filter(pred::Header("Content-Type", "text/plain"))
    ///                    .f(|_| HttpResponse::Ok())
    ///             })
    ///         });
    /// }
    /// ```
    pub fn resource<F, R>(mut self, path: &str, f: F) -> Scope<S>
    where
        F: FnOnce(&mut ResourceHandler<S>) -> R + 'static,
    {
        // add resource handler
        let mut handler = ResourceHandler::default();
        f(&mut handler);

        let pattern = Resource::new(handler.get_name(), path);
        Rc::get_mut(&mut self.resources)
            .expect("Can not use after configuration")
            .push((pattern, Rc::new(UnsafeCell::new(handler))));

        self
    }

    /// Default resource to be used if no matching route could be found.
    pub fn default_resource<F, R>(self, f: F) -> Scope<S>
    where
        F: FnOnce(&mut ResourceHandler<S>) -> R + 'static,
    {
        let default = unsafe { &mut *self.default.as_ref().get() };
        f(default);
        self
    }

    /// Register a scope middleware
    ///
    /// This is similar to `App's` middlewares, but
    /// middlewares get invoked on scope level.
    ///
    /// *Note* `Middleware::finish()` fires right after response get
    /// prepared. It does not wait until body get sent to peer.
    pub fn middleware<M: Middleware<S>>(mut self, mw: M) -> Scope<S> {
        Rc::get_mut(&mut self.middlewares)
            .expect("Can not use after configuration")
            .push(Box::new(mw));
        self
    }
}

impl<S: 'static> RouteHandler<S> for Scope<S> {
    fn handle(&mut self, mut req: HttpRequest<S>) -> Reply {
        let path = unsafe { &*(&req.match_info()["tail"] as *const _) };
        let path = if path == "" { "/" } else { path };

        // recognize paths
        for &(ref pattern, ref resource) in self.resources.iter() {
            if pattern.match_with_params(path, req.match_info_mut()) {
                let default = unsafe { &mut *self.default.as_ref().get() };

                if self.middlewares.is_empty() {
                    let resource = unsafe { &mut *resource.get() };
                    return resource.handle(req, Some(default));
                } else {
                    return Reply::async(Compose::new(
                        req,
                        Rc::clone(&self.middlewares),
                        Rc::clone(&resource),
                        Some(Rc::clone(&self.default)),
                    ));
                }
            }
        }

        // nested scope
        if let Some(ref handler) = self.handler {
            let hnd: &mut RouteHandler<_> = unsafe { (&mut *(handler.get())).as_mut() };
            hnd.handle(req)
        } else {
            // default handler
            let default = unsafe { &mut *self.default.as_ref().get() };
            if self.middlewares.is_empty() {
                default.handle(req, None)
            } else {
                Reply::async(Compose::new(
                    req,
                    Rc::clone(&self.middlewares),
                    Rc::clone(&self.default),
                    None,
                ))
            }
        }
    }
}

struct Wrapper<S: 'static> {
    state: Rc<S>,
    scope: Scope<S>,
}

impl<S: 'static, S2: 'static> RouteHandler<S2> for Wrapper<S> {
    fn handle(&mut self, req: HttpRequest<S2>) -> Reply {
        self.scope
            .handle(req.change_state(Rc::clone(&self.state)))
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
    default: Option<Rc<UnsafeCell<ResourceHandler<S>>>>,
    resource: Rc<UnsafeCell<ResourceHandler<S>>>,
}

enum ComposeState<S: 'static> {
    Starting(StartMiddlewares<S>),
    Handler(WaitingResponse<S>),
    RunMiddlewares(RunMiddlewares<S>),
    Finishing(FinishingMiddlewares<S>),
    Completed(Response<S>),
}

impl<S: 'static> ComposeState<S> {
    fn poll(&mut self, info: &mut ComposeInfo<S>) -> Option<ComposeState<S>> {
        match *self {
            ComposeState::Starting(ref mut state) => state.poll(info),
            ComposeState::Handler(ref mut state) => state.poll(info),
            ComposeState::RunMiddlewares(ref mut state) => state.poll(info),
            ComposeState::Finishing(ref mut state) => state.poll(info),
            ComposeState::Completed(_) => None,
        }
    }
}

impl<S: 'static> Compose<S> {
    fn new(
        req: HttpRequest<S>, mws: Rc<Vec<Box<Middleware<S>>>>,
        resource: Rc<UnsafeCell<ResourceHandler<S>>>,
        default: Option<Rc<UnsafeCell<ResourceHandler<S>>>>,
    ) -> Self {
        let mut info = ComposeInfo {
            count: 0,
            req,
            mws,
            resource,
            default,
        };
        let state = StartMiddlewares::init(&mut info);

        Compose { state, info }
    }
}

impl<S> Future for Compose<S> {
    type Item = HttpResponse;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            if let ComposeState::Completed(ref mut resp) = self.state {
                let resp = resp.resp.take().unwrap();
                return Ok(Async::Ready(resp));
            }
            if let Some(state) = self.state.poll(&mut self.info) {
                self.state = state;
            } else {
                return Ok(Async::NotReady);
            }
        }
    }
}

/// Middlewares start executor
struct StartMiddlewares<S> {
    fut: Option<Fut>,
    _s: PhantomData<S>,
}

type Fut = Box<Future<Item = Option<HttpResponse>, Error = Error>>;

impl<S: 'static> StartMiddlewares<S> {
    fn init(info: &mut ComposeInfo<S>) -> ComposeState<S> {
        let len = info.mws.len();
        loop {
            if info.count == len {
                let resource = unsafe { &mut *info.resource.get() };
                let reply = if let Some(ref default) = info.default {
                    let d = unsafe { &mut *default.as_ref().get() };
                    resource.handle(info.req.clone(), Some(d))
                } else {
                    resource.handle(info.req.clone(), None)
                };
                return WaitingResponse::init(info, reply);
            } else {
                match info.mws[info.count].start(&mut info.req) {
                    Ok(MiddlewareStarted::Done) => info.count += 1,
                    Ok(MiddlewareStarted::Response(resp)) => {
                        return RunMiddlewares::init(info, resp)
                    }
                    Ok(MiddlewareStarted::Future(mut fut)) => match fut.poll() {
                        Ok(Async::NotReady) => {
                            return ComposeState::Starting(StartMiddlewares {
                                fut: Some(fut),
                                _s: PhantomData,
                            })
                        }
                        Ok(Async::Ready(resp)) => {
                            if let Some(resp) = resp {
                                return RunMiddlewares::init(info, resp);
                            }
                            info.count += 1;
                        }
                        Err(err) => return Response::init(err.into()),
                    },
                    Err(err) => return Response::init(err.into()),
                }
            }
        }
    }

    fn poll(&mut self, info: &mut ComposeInfo<S>) -> Option<ComposeState<S>> {
        let len = info.mws.len();
        'outer: loop {
            match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) => return None,
                Ok(Async::Ready(resp)) => {
                    info.count += 1;
                    if let Some(resp) = resp {
                        return Some(RunMiddlewares::init(info, resp));
                    }
                    if info.count == len {
                        let resource = unsafe { &mut *info.resource.get() };
                        let reply = if let Some(ref default) = info.default {
                            let d = unsafe { &mut *default.as_ref().get() };
                            resource.handle(info.req.clone(), Some(d))
                        } else {
                            resource.handle(info.req.clone(), None)
                        };
                        return Some(WaitingResponse::init(info, reply));
                    } else {
                        loop {
                            match info.mws[info.count].start(&mut info.req) {
                                Ok(MiddlewareStarted::Done) => info.count += 1,
                                Ok(MiddlewareStarted::Response(resp)) => {
                                    return Some(RunMiddlewares::init(info, resp));
                                }
                                Ok(MiddlewareStarted::Future(fut)) => {
                                    self.fut = Some(fut);
                                    continue 'outer;
                                }
                                Err(err) => return Some(Response::init(err.into())),
                            }
                        }
                    }
                }
                Err(err) => return Some(Response::init(err.into())),
            }
        }
    }
}

// waiting for response
struct WaitingResponse<S> {
    fut: Box<Future<Item = HttpResponse, Error = Error>>,
    _s: PhantomData<S>,
}

impl<S: 'static> WaitingResponse<S> {
    #[inline]
    fn init(info: &mut ComposeInfo<S>, reply: Reply) -> ComposeState<S> {
        match reply.into() {
            ReplyItem::Message(resp) => RunMiddlewares::init(info, resp),
            ReplyItem::Future(fut) => ComposeState::Handler(WaitingResponse {
                fut,
                _s: PhantomData,
            }),
        }
    }

    fn poll(&mut self, info: &mut ComposeInfo<S>) -> Option<ComposeState<S>> {
        match self.fut.poll() {
            Ok(Async::NotReady) => None,
            Ok(Async::Ready(response)) => Some(RunMiddlewares::init(info, response)),
            Err(err) => Some(RunMiddlewares::init(info, err.into())),
        }
    }
}

/// Middlewares response executor
struct RunMiddlewares<S> {
    curr: usize,
    fut: Option<Box<Future<Item = HttpResponse, Error = Error>>>,
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
                    return FinishingMiddlewares::init(info, err.into());
                }
                Ok(MiddlewareResponse::Done(r)) => {
                    curr += 1;
                    if curr == len {
                        return FinishingMiddlewares::init(info, r);
                    } else {
                        r
                    }
                }
                Ok(MiddlewareResponse::Future(fut)) => {
                    return ComposeState::RunMiddlewares(RunMiddlewares {
                        curr,
                        fut: Some(fut),
                        _s: PhantomData,
                    })
                }
            };
        }
    }

    fn poll(&mut self, info: &mut ComposeInfo<S>) -> Option<ComposeState<S>> {
        let len = info.mws.len();

        loop {
            // poll latest fut
            let mut resp = match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) => return None,
                Ok(Async::Ready(resp)) => {
                    self.curr += 1;
                    resp
                }
                Err(err) => return Some(FinishingMiddlewares::init(info, err.into())),
            };

            loop {
                if self.curr == len {
                    return Some(FinishingMiddlewares::init(info, resp));
                } else {
                    match info.mws[self.curr].response(&mut info.req, resp) {
                        Err(err) => {
                            return Some(FinishingMiddlewares::init(info, err.into()))
                        }
                        Ok(MiddlewareResponse::Done(r)) => {
                            self.curr += 1;
                            resp = r
                        }
                        Ok(MiddlewareResponse::Future(fut)) => {
                            self.fut = Some(fut);
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// Middlewares start executor
struct FinishingMiddlewares<S> {
    resp: Option<HttpResponse>,
    fut: Option<Box<Future<Item = (), Error = Error>>>,
    _s: PhantomData<S>,
}

impl<S: 'static> FinishingMiddlewares<S> {
    fn init(info: &mut ComposeInfo<S>, resp: HttpResponse) -> ComposeState<S> {
        if info.count == 0 {
            Response::init(resp)
        } else {
            let mut state = FinishingMiddlewares {
                resp: Some(resp),
                fut: None,
                _s: PhantomData,
            };
            if let Some(st) = state.poll(info) {
                st
            } else {
                ComposeState::Finishing(state)
            }
        }
    }

    fn poll(&mut self, info: &mut ComposeInfo<S>) -> Option<ComposeState<S>> {
        loop {
            // poll latest fut
            let not_ready = if let Some(ref mut fut) = self.fut {
                match fut.poll() {
                    Ok(Async::NotReady) => true,
                    Ok(Async::Ready(())) => false,
                    Err(err) => {
                        error!("Middleware finish error: {}", err);
                        false
                    }
                }
            } else {
                false
            };
            if not_ready {
                return None;
            }
            self.fut = None;
            info.count -= 1;

            match info.mws[info.count as usize]
                .finish(&mut info.req, self.resp.as_ref().unwrap())
            {
                MiddlewareFinished::Done => {
                    if info.count == 0 {
                        return Some(Response::init(self.resp.take().unwrap()));
                    }
                }
                MiddlewareFinished::Future(fut) => {
                    self.fut = Some(fut);
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
        ComposeState::Completed(Response {
            resp: Some(resp),
            _s: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use application::App;
    use http::StatusCode;
    use httpresponse::HttpResponse;
    use test::TestRequest;

    #[test]
    fn test_scope() {
        let mut app = App::new()
            .scope("/app", |scope| {
                scope.resource("/path1", |r| r.f(|_| HttpResponse::Ok()))
            })
            .finish();

        let req = TestRequest::with_uri("/app/path1").finish();
        let resp = app.run(req);
        assert_eq!(resp.as_response().unwrap().status(), StatusCode::OK);
    }

    #[test]
    fn test_scope_with_state() {
        struct State;

        let mut app = App::new()
            .scope("/app", |scope| {
                scope.with_state(State, |scope| {
                    scope.resource("/path1", |r| r.f(|_| HttpResponse::Created()))
                })
            })
            .finish();

        let req = TestRequest::with_uri("/app/path1").finish();
        let resp = app.run(req);
        assert_eq!(
            resp.as_response().unwrap().status(),
            StatusCode::CREATED
        );
    }

    #[test]
    fn test_default_resource() {
        let mut app = App::new()
            .scope("/app", |scope| {
                scope
                    .resource("/path1", |r| r.f(|_| HttpResponse::Ok()))
                    .default_resource(|r| r.f(|_| HttpResponse::BadRequest()))
            })
            .finish();

        let req = TestRequest::with_uri("/app/path2").finish();
        let resp = app.run(req);
        assert_eq!(
            resp.as_response().unwrap().status(),
            StatusCode::BAD_REQUEST
        );

        let req = TestRequest::with_uri("/path2").finish();
        let resp = app.run(req);
        assert_eq!(
            resp.as_response().unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
}
