use std::marker::PhantomData;
use std::mem;
use std::rc::Rc;

use futures::{Async, Future, Poll};

use error::Error;
use handler::{
    AsyncResult, AsyncResultItem, FromRequest, Handler, Responder, RouteHandler,
    WrapHandler,
};
use http::Method;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::{
    Finished as MiddlewareFinished, Middleware, Response as MiddlewareResponse,
    Started as MiddlewareStarted,
};
use pred::Predicate;
use resource::{DefaultResource, Resource};
use router::{ResourceDef, Router};
use server::Request;
use with::WithFactory;

/// Resources scope
///
/// Scope is a set of resources with common root path.
/// Scopes collect multiple paths under a common path prefix.
/// Scope path can contain variable path segments as resources.
/// Scope prefix is always complete path segment, i.e `/app` would
/// be converted to a `/app/` and it would not match `/app` path.
///
/// You can get variable path segments from `HttpRequest::match_info()`.
/// `Path` extractor also is able to extract scope level variable segments.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{http, App, HttpRequest, HttpResponse};
///
/// fn main() {
///     let app = App::new().scope("/{project_id}/", |scope| {
///         scope
///             .resource("/path1", |r| r.f(|_| HttpResponse::Ok()))
///             .resource("/path2", |r| r.f(|_| HttpResponse::Ok()))
///             .resource("/path3", |r| r.f(|_| HttpResponse::MethodNotAllowed()))
///     });
/// }
/// ```
///
/// In the above example three routes get registered:
///  * /{project_id}/path1 - reponds to all http method
///  * /{project_id}/path2 - `GET` requests
///  * /{project_id}/path3 - `HEAD` requests
///
pub struct Scope<S> {
    rdef: ResourceDef,
    router: Rc<Router<S>>,
    filters: Vec<Box<Predicate<S>>>,
    middlewares: Rc<Vec<Box<Middleware<S>>>>,
}

#[cfg_attr(
    feature = "cargo-clippy",
    allow(new_without_default_derive)
)]
impl<S: 'static> Scope<S> {
    /// Create a new scope
    pub fn new(path: &str) -> Scope<S> {
        let rdef = ResourceDef::prefix(path);
        Scope {
            rdef: rdef.clone(),
            router: Rc::new(Router::new(rdef)),
            filters: Vec::new(),
            middlewares: Rc::new(Vec::new()),
        }
    }

    #[inline]
    pub(crate) fn rdef(&self) -> &ResourceDef {
        &self.rdef
    }

    pub(crate) fn router(&self) -> &Router<S> {
        self.router.as_ref()
    }

    #[inline]
    pub(crate) fn take_filters(&mut self) -> Vec<Box<Predicate<S>>> {
        mem::replace(&mut self.filters, Vec::new())
    }

    /// Add match predicate to scope.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, pred, App, HttpRequest, HttpResponse, Path};
    ///
    /// fn index(data: Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().scope("/app", |scope| {
    ///         scope
    ///             .filter(pred::Header("content-type", "text/plain"))
    ///             .route("/test1", http::Method::GET, index)
    ///             .route("/test2", http::Method::POST, |_: HttpRequest| {
    ///                 HttpResponse::MethodNotAllowed()
    ///             })
    ///     });
    /// }
    /// ```
    pub fn filter<T: Predicate<S> + 'static>(mut self, p: T) -> Self {
        self.filters.push(Box::new(p));
        self
    }

    /// Create nested scope with new state.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{App, HttpRequest};
    ///
    /// struct AppState;
    ///
    /// fn index(req: &HttpRequest<AppState>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().scope("/app", |scope| {
    ///         scope.with_state("/state2", AppState, |scope| {
    ///             scope.resource("/test1", |r| r.f(index))
    ///         })
    ///     });
    /// }
    /// ```
    pub fn with_state<F, T: 'static>(mut self, path: &str, state: T, f: F) -> Scope<S>
    where
        F: FnOnce(Scope<T>) -> Scope<T>,
    {
        let rdef = ResourceDef::prefix(path);
        let scope = Scope {
            rdef: rdef.clone(),
            filters: Vec::new(),
            router: Rc::new(Router::new(rdef)),
            middlewares: Rc::new(Vec::new()),
        };
        let mut scope = f(scope);

        let state = Rc::new(state);
        let filters: Vec<Box<Predicate<S>>> = vec![Box::new(FiltersWrapper {
            state: Rc::clone(&state),
            filters: scope.take_filters(),
        })];
        let handler = Box::new(Wrapper { scope, state });

        Rc::get_mut(&mut self.router).unwrap().register_handler(
            path,
            handler,
            Some(filters),
        );

        self
    }

    /// Create nested scope.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{App, HttpRequest};
    ///
    /// struct AppState;
    ///
    /// fn index(req: &HttpRequest<AppState>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::with_state(AppState).scope("/app", |scope| {
    ///         scope.nested("/v1", |scope| scope.resource("/test1", |r| r.f(index)))
    ///     });
    /// }
    /// ```
    pub fn nested<F>(mut self, path: &str, f: F) -> Scope<S>
    where
        F: FnOnce(Scope<S>) -> Scope<S>,
    {
        let rdef = ResourceDef::prefix(&insert_slash(path));
        let scope = Scope {
            rdef: rdef.clone(),
            filters: Vec::new(),
            router: Rc::new(Router::new(rdef)),
            middlewares: Rc::new(Vec::new()),
        };
        Rc::get_mut(&mut self.router)
            .unwrap()
            .register_scope(f(scope));

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
    ///     let app = App::new().scope("/app", |scope| {
    ///         scope.route("/test1", http::Method::GET, index).route(
    ///             "/test2",
    ///             http::Method::POST,
    ///             |_: HttpRequest| HttpResponse::MethodNotAllowed(),
    ///         )
    ///     });
    /// }
    /// ```
    pub fn route<T, F, R>(mut self, path: &str, method: Method, f: F) -> Scope<S>
    where
        F: WithFactory<T, S, R>,
        R: Responder + 'static,
        T: FromRequest<S> + 'static,
    {
        Rc::get_mut(&mut self.router).unwrap().register_route(
            &insert_slash(path),
            method,
            f,
        );
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
    ///     let app = App::new().scope("/api", |scope| {
    ///         scope.resource("/users/{userid}/{friend}", |r| {
    ///             r.get().f(|_| HttpResponse::Ok());
    ///             r.head().f(|_| HttpResponse::MethodNotAllowed());
    ///             r.route()
    ///                 .filter(pred::Any(pred::Get()).or(pred::Put()))
    ///                 .filter(pred::Header("Content-Type", "text/plain"))
    ///                 .f(|_| HttpResponse::Ok())
    ///         })
    ///     });
    /// }
    /// ```
    pub fn resource<F, R>(mut self, path: &str, f: F) -> Scope<S>
    where
        F: FnOnce(&mut Resource<S>) -> R + 'static,
    {
        // add resource
        let mut resource = Resource::new(ResourceDef::new(&insert_slash(path)));
        f(&mut resource);

        Rc::get_mut(&mut self.router)
            .unwrap()
            .register_resource(resource);
        self
    }

    /// Default resource to be used if no matching route could be found.
    pub fn default_resource<F, R>(mut self, f: F) -> Scope<S>
    where
        F: FnOnce(&mut Resource<S>) -> R + 'static,
    {
        // create and configure default resource
        let mut resource = Resource::new(ResourceDef::new(""));
        f(&mut resource);

        Rc::get_mut(&mut self.router)
            .expect("Multiple copies of scope router")
            .register_default_resource(resource.into());

        self
    }

    /// Configure handler for specific path prefix.
    ///
    /// A path prefix consists of valid path segments, i.e for the
    /// prefix `/app` any request with the paths `/app`, `/app/` or
    /// `/app/test` would match, but the path `/application` would
    /// not.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{http, App, HttpRequest, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().scope("/scope-prefix", |scope| {
    ///         scope.handler("/app", |req: &HttpRequest| match *req.method() {
    ///             http::Method::GET => HttpResponse::Ok(),
    ///             http::Method::POST => HttpResponse::MethodNotAllowed(),
    ///             _ => HttpResponse::NotFound(),
    ///         })
    ///     });
    /// }
    /// ```
    pub fn handler<H: Handler<S>>(mut self, path: &str, handler: H) -> Scope<S> {
        let path = insert_slash(path.trim().trim_right_matches('/'));
        Rc::get_mut(&mut self.router)
            .expect("Multiple copies of scope router")
            .register_handler(&path, Box::new(WrapHandler::new(handler)), None);
        self
    }

    /// Register a scope middleware
    ///
    /// This is similar to `App's` middlewares, but
    /// middlewares get invoked on scope level.
    ///
    /// *Note* `Middleware::finish()` fires right after response get
    /// prepared. It does not wait until body get sent to the peer.
    pub fn middleware<M: Middleware<S>>(mut self, mw: M) -> Scope<S> {
        Rc::get_mut(&mut self.middlewares)
            .expect("Can not use after configuration")
            .push(Box::new(mw));
        self
    }
}

fn insert_slash(path: &str) -> String {
    let mut path = path.to_owned();
    if !path.is_empty() && !path.starts_with('/') {
        path.insert(0, '/');
    };
    path
}

impl<S: 'static> RouteHandler<S> for Scope<S> {
    fn handle(&self, req: &HttpRequest<S>) -> AsyncResult<HttpResponse> {
        let tail = req.match_info().tail as usize;

        // recognize resources
        let info = self.router.recognize(req, req.state(), tail);
        let req2 = req.with_route_info(info);
        if self.middlewares.is_empty() {
            self.router.handle(&req2)
        } else {
            AsyncResult::async(Box::new(Compose::new(
                req2,
                Rc::clone(&self.router),
                Rc::clone(&self.middlewares),
            )))
        }
    }

    fn has_default_resource(&self) -> bool {
        self.router.has_default_resource()
    }

    fn default_resource(&mut self, default: DefaultResource<S>) {
        Rc::get_mut(&mut self.router)
            .expect("Can not use after configuration")
            .register_default_resource(default);
    }

    fn finish(&mut self) {
        Rc::get_mut(&mut self.router)
            .expect("Can not use after configuration")
            .finish();
    }
}

struct Wrapper<S: 'static> {
    state: Rc<S>,
    scope: Scope<S>,
}

impl<S: 'static, S2: 'static> RouteHandler<S2> for Wrapper<S> {
    fn handle(&self, req: &HttpRequest<S2>) -> AsyncResult<HttpResponse> {
        let req = req.with_state(Rc::clone(&self.state));
        self.scope.handle(&req)
    }
}

struct FiltersWrapper<S: 'static> {
    state: Rc<S>,
    filters: Vec<Box<Predicate<S>>>,
}

impl<S: 'static, S2: 'static> Predicate<S2> for FiltersWrapper<S> {
    fn check(&self, req: &Request, _: &S2) -> bool {
        for filter in &self.filters {
            if !filter.check(&req, &self.state) {
                return false;
            }
        }
        true
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
    router: Rc<Router<S>>,
    mws: Rc<Vec<Box<Middleware<S>>>>,
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
        req: HttpRequest<S>, router: Rc<Router<S>>, mws: Rc<Vec<Box<Middleware<S>>>>,
    ) -> Self {
        let mut info = ComposeInfo {
            mws,
            req,
            router,
            count: 0,
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
                let reply = info.router.handle(&info.req);
                return WaitingResponse::init(info, reply);
            } else {
                let result = info.mws[info.count].start(&info.req);
                match result {
                    Ok(MiddlewareStarted::Done) => info.count += 1,
                    Ok(MiddlewareStarted::Response(resp)) => {
                        return RunMiddlewares::init(info, resp);
                    }
                    Ok(MiddlewareStarted::Future(fut)) => {
                        return ComposeState::Starting(StartMiddlewares {
                            fut: Some(fut),
                            _s: PhantomData,
                        });
                    }
                    Err(err) => {
                        return RunMiddlewares::init(info, err.into());
                    }
                }
            }
        }
    }

    fn poll(&mut self, info: &mut ComposeInfo<S>) -> Option<ComposeState<S>> {
        let len = info.mws.len();

        'outer: loop {
            match self.fut.as_mut().unwrap().poll() {
                Ok(Async::NotReady) => {
                    return None;
                }
                Ok(Async::Ready(resp)) => {
                    info.count += 1;

                    if let Some(resp) = resp {
                        return Some(RunMiddlewares::init(info, resp));
                    }
                    loop {
                        if info.count == len {
                            let reply = info.router.handle(&info.req);
                            return Some(WaitingResponse::init(info, reply));
                        } else {
                            let result = info.mws[info.count].start(&info.req);
                            match result {
                                Ok(MiddlewareStarted::Done) => info.count += 1,
                                Ok(MiddlewareStarted::Response(resp)) => {
                                    return Some(RunMiddlewares::init(info, resp));
                                }
                                Ok(MiddlewareStarted::Future(fut)) => {
                                    self.fut = Some(fut);
                                    continue 'outer;
                                }
                                Err(err) => {
                                    return Some(RunMiddlewares::init(info, err.into()));
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    return Some(RunMiddlewares::init(info, err.into()));
                }
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
    fn init(
        info: &mut ComposeInfo<S>, reply: AsyncResult<HttpResponse>,
    ) -> ComposeState<S> {
        match reply.into() {
            AsyncResultItem::Ok(resp) => RunMiddlewares::init(info, resp),
            AsyncResultItem::Err(err) => RunMiddlewares::init(info, err.into()),
            AsyncResultItem::Future(fut) => ComposeState::Handler(WaitingResponse {
                fut,
                _s: PhantomData,
            }),
        }
    }

    fn poll(&mut self, info: &mut ComposeInfo<S>) -> Option<ComposeState<S>> {
        match self.fut.poll() {
            Ok(Async::NotReady) => None,
            Ok(Async::Ready(resp)) => Some(RunMiddlewares::init(info, resp)),
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
            let state = info.mws[curr].response(&info.req, resp);
            resp = match state {
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
                    });
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
                    let state = info.mws[self.curr].response(&info.req, resp);
                    match state {
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
            if info.count == 0 {
                return Some(Response::init(self.resp.take().unwrap()));
            }

            info.count -= 1;
            let state = info.mws[info.count as usize]
                .finish(&info.req, self.resp.as_ref().unwrap());
            match state {
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
    use bytes::Bytes;

    use application::App;
    use body::Body;
    use http::{Method, StatusCode};
    use httprequest::HttpRequest;
    use httpresponse::HttpResponse;
    use pred;
    use test::TestRequest;

    #[test]
    fn test_scope() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.resource("/path1", |r| r.f(|_| HttpResponse::Ok()))
            }).finish();

        let req = TestRequest::with_uri("/app/path1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);
    }

    #[test]
    fn test_scope_root() {
        let app = App::new()
            .scope("/app", |scope| {
                scope
                    .resource("", |r| r.f(|_| HttpResponse::Ok()))
                    .resource("/", |r| r.f(|_| HttpResponse::Created()))
            }).finish();

        let req = TestRequest::with_uri("/app").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::CREATED);
    }

    #[test]
    fn test_scope_root2() {
        let app = App::new()
            .scope("/app/", |scope| {
                scope.resource("", |r| r.f(|_| HttpResponse::Ok()))
            }).finish();

        let req = TestRequest::with_uri("/app").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);
    }

    #[test]
    fn test_scope_root3() {
        let app = App::new()
            .scope("/app/", |scope| {
                scope.resource("/", |r| r.f(|_| HttpResponse::Ok()))
            }).finish();

        let req = TestRequest::with_uri("/app").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_scope_route() {
        let app = App::new()
            .scope("app", |scope| {
                scope
                    .route("/path1", Method::GET, |_: HttpRequest<_>| {
                        HttpResponse::Ok()
                    }).route("/path1", Method::DELETE, |_: HttpRequest<_>| {
                        HttpResponse::Ok()
                    })
            }).finish();

        let req = TestRequest::with_uri("/app/path1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::DELETE)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_scope_route_without_leading_slash() {
        let app = App::new()
            .scope("app", |scope| {
                scope
                    .route("path1", Method::GET, |_: HttpRequest<_>| HttpResponse::Ok())
                    .route("path1", Method::DELETE, |_: HttpRequest<_>| {
                        HttpResponse::Ok()
                    })
            }).finish();

        let req = TestRequest::with_uri("/app/path1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::DELETE)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_scope_filter() {
        let app = App::new()
            .scope("/app", |scope| {
                scope
                    .filter(pred::Get())
                    .resource("/path1", |r| r.f(|_| HttpResponse::Ok()))
            }).finish();

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::GET)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);
    }

    #[test]
    fn test_scope_variable_segment() {
        let app = App::new()
            .scope("/ab-{project}", |scope| {
                scope.resource("/path1", |r| {
                    r.f(|r| {
                        HttpResponse::Ok()
                            .body(format!("project: {}", &r.match_info()["project"]))
                    })
                })
            }).finish();

        let req = TestRequest::with_uri("/ab-project1/path1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        match resp.as_msg().body() {
            &Body::Binary(ref b) => {
                let bytes: Bytes = b.clone().into();
                assert_eq!(bytes, Bytes::from_static(b"project: project1"));
            }
            _ => panic!(),
        }

        let req = TestRequest::with_uri("/aa-project1/path1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_scope_with_state() {
        struct State;

        let app = App::new()
            .scope("/app", |scope| {
                scope.with_state("/t1", State, |scope| {
                    scope.resource("/path1", |r| r.f(|_| HttpResponse::Created()))
                })
            }).finish();

        let req = TestRequest::with_uri("/app/t1/path1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::CREATED);
    }

    #[test]
    fn test_scope_with_state_root() {
        struct State;

        let app = App::new()
            .scope("/app", |scope| {
                scope.with_state("/t1", State, |scope| {
                    scope
                        .resource("", |r| r.f(|_| HttpResponse::Ok()))
                        .resource("/", |r| r.f(|_| HttpResponse::Created()))
                })
            }).finish();

        let req = TestRequest::with_uri("/app/t1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/t1/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::CREATED);
    }

    #[test]
    fn test_scope_with_state_root2() {
        struct State;

        let app = App::new()
            .scope("/app", |scope| {
                scope.with_state("/t1/", State, |scope| {
                    scope.resource("", |r| r.f(|_| HttpResponse::Ok()))
                })
            }).finish();

        let req = TestRequest::with_uri("/app/t1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/t1/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);
    }

    #[test]
    fn test_scope_with_state_root3() {
        struct State;

        let app = App::new()
            .scope("/app", |scope| {
                scope.with_state("/t1/", State, |scope| {
                    scope.resource("/", |r| r.f(|_| HttpResponse::Ok()))
                })
            }).finish();

        let req = TestRequest::with_uri("/app/t1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/t1/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_scope_with_state_filter() {
        struct State;

        let app = App::new()
            .scope("/app", |scope| {
                scope.with_state("/t1", State, |scope| {
                    scope
                        .filter(pred::Get())
                        .resource("/path1", |r| r.f(|_| HttpResponse::Ok()))
                })
            }).finish();

        let req = TestRequest::with_uri("/app/t1/path1")
            .method(Method::POST)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/t1/path1")
            .method(Method::GET)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);
    }

    #[test]
    fn test_nested_scope() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("/t1", |scope| {
                    scope.resource("/path1", |r| r.f(|_| HttpResponse::Created()))
                })
            }).finish();

        let req = TestRequest::with_uri("/app/t1/path1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::CREATED);
    }

    #[test]
    fn test_nested_scope_no_slash() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("t1", |scope| {
                    scope.resource("/path1", |r| r.f(|_| HttpResponse::Created()))
                })
            }).finish();

        let req = TestRequest::with_uri("/app/t1/path1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::CREATED);
    }

    #[test]
    fn test_nested_scope_root() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("/t1", |scope| {
                    scope
                        .resource("", |r| r.f(|_| HttpResponse::Ok()))
                        .resource("/", |r| r.f(|_| HttpResponse::Created()))
                })
            }).finish();

        let req = TestRequest::with_uri("/app/t1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/t1/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::CREATED);
    }

    #[test]
    fn test_nested_scope_filter() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("/t1", |scope| {
                    scope
                        .filter(pred::Get())
                        .resource("/path1", |r| r.f(|_| HttpResponse::Ok()))
                })
            }).finish();

        let req = TestRequest::with_uri("/app/t1/path1")
            .method(Method::POST)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/t1/path1")
            .method(Method::GET)
            .request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);
    }

    #[test]
    fn test_nested_scope_with_variable_segment() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("/{project_id}", |scope| {
                    scope.resource("/path1", |r| {
                        r.f(|r| {
                            HttpResponse::Created().body(format!(
                                "project: {}",
                                &r.match_info()["project_id"]
                            ))
                        })
                    })
                })
            }).finish();

        let req = TestRequest::with_uri("/app/project_1/path1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::CREATED);

        match resp.as_msg().body() {
            &Body::Binary(ref b) => {
                let bytes: Bytes = b.clone().into();
                assert_eq!(bytes, Bytes::from_static(b"project: project_1"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_nested2_scope_with_variable_segment() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("/{project}", |scope| {
                    scope.nested("/{id}", |scope| {
                        scope.resource("/path1", |r| {
                            r.f(|r| {
                                HttpResponse::Created().body(format!(
                                    "project: {} - {}",
                                    &r.match_info()["project"],
                                    &r.match_info()["id"],
                                ))
                            })
                        })
                    })
                })
            }).finish();

        let req = TestRequest::with_uri("/app/test/1/path1").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::CREATED);

        match resp.as_msg().body() {
            &Body::Binary(ref b) => {
                let bytes: Bytes = b.clone().into();
                assert_eq!(bytes, Bytes::from_static(b"project: test - 1"));
            }
            _ => panic!(),
        }

        let req = TestRequest::with_uri("/app/test/1/path2").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_default_resource() {
        let app = App::new()
            .scope("/app", |scope| {
                scope
                    .resource("/path1", |r| r.f(|_| HttpResponse::Ok()))
                    .default_resource(|r| r.f(|_| HttpResponse::BadRequest()))
            }).finish();

        let req = TestRequest::with_uri("/app/path2").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/path2").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_default_resource_propagation() {
        let app = App::new()
            .scope("/app1", |scope| {
                scope.default_resource(|r| r.f(|_| HttpResponse::BadRequest()))
            }).scope("/app2", |scope| scope)
            .default_resource(|r| r.f(|_| HttpResponse::MethodNotAllowed()))
            .finish();

        let req = TestRequest::with_uri("/non-exist").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/app1/non-exist").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/app2/non-exist").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn test_handler() {
        let app = App::new()
            .scope("/scope", |scope| {
                scope.handler("/test", |_: &_| HttpResponse::Ok())
            }).finish();

        let req = TestRequest::with_uri("/scope/test").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/scope/test/").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/scope/test/app").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::OK);

        let req = TestRequest::with_uri("/scope/testapp").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/scope/blah").request();
        let resp = app.run(req);
        assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    }
}
