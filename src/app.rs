use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::body::{Body, MessageBody};
use actix_http::{Extensions, PayloadStream, Request, Response};
use actix_router::{Path, ResourceDef, ResourceInfo, Router, Url};
use actix_service::boxed::{self, BoxedNewService, BoxedService};
use actix_service::{
    AndThenNewService, ApplyTransform, IntoNewService, IntoTransform, NewService,
    Service, Transform,
};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, IntoFuture, Poll};

use crate::guard::Guard;
use crate::resource::Resource;
use crate::scope::{insert_slash, Scope};
use crate::service::{ServiceRequest, ServiceResponse};
use crate::state::{State, StateFactory, StateFactoryResult};

type Guards = Vec<Box<Guard>>;
type HttpService<P> = BoxedService<ServiceRequest<P>, ServiceResponse, ()>;
type HttpNewService<P> = BoxedNewService<(), ServiceRequest<P>, ServiceResponse, (), ()>;
type BoxedResponse = Box<Future<Item = ServiceResponse, Error = ()>>;

pub trait HttpServiceFactory<Request> {
    type Factory: NewService<Request = Request>;

    fn rdef(&self) -> &ResourceDef;

    fn create(self) -> Self::Factory;
}

/// Application builder - structure that follows the builder pattern
/// for building application instances.
pub struct App<P, T>
where
    T: NewService<Request = ServiceRequest<PayloadStream>, Response = ServiceRequest<P>>,
{
    chain: T,
    extensions: Extensions,
    state: Vec<Box<StateFactory>>,
    _t: PhantomData<(P,)>,
}

impl App<PayloadStream, AppChain> {
    /// Create application builder with empty state. Application can
    /// be configured with a builder-like pattern.
    pub fn new() -> Self {
        App {
            chain: AppChain,
            extensions: Extensions::new(),
            state: Vec::new(),
            _t: PhantomData,
        }
    }
}

impl<P, T> App<P, T>
where
    P: 'static,
    T: NewService<
        Request = ServiceRequest<PayloadStream>,
        Response = ServiceRequest<P>,
        Error = (),
        InitError = (),
    >,
{
    /// Set application state. Applicatin state could be accessed
    /// by using `State<T>` extractor where `T` is state type.
    ///
    /// **Note**: http server accepts an application factory rather than
    /// an application instance. Http server constructs an application
    /// instance for each thread, thus application state must be constructed
    /// multiple times. If you want to share state between different
    /// threads, a shared object should be used, e.g. `Arc`. Application
    /// state does not need to be `Send` or `Sync`.
    ///
    /// ```rust
    /// use std::cell::Cell;
    /// use actix_web::{web, State, App};
    ///
    /// struct MyState {
    ///     counter: Cell<usize>,
    /// }
    ///
    /// fn index(state: State<MyState>) {
    ///     state.counter.set(state.counter.get() + 1);
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .state(MyState{ counter: Cell::new(0) })
    ///         .resource(
    ///             "/index.html",
    ///             |r| r.route(web::get().to(index)));
    /// }
    /// ```
    pub fn state<S: 'static>(mut self, state: S) -> Self {
        self.state.push(Box::new(State::new(state)));
        self
    }

    /// Set application state factory. This function is
    /// similar to `.state()` but it accepts state factory. State get
    /// constructed asynchronously during application initialization.
    pub fn state_factory<F, Out>(mut self, state: F) -> Self
    where
        F: Fn() -> Out + 'static,
        Out: IntoFuture + 'static,
        Out::Error: std::fmt::Debug,
    {
        self.state.push(Box::new(state));
        self
    }

    /// Configure scope for common root path.
    ///
    /// Scopes collect multiple paths under a common path prefix.
    /// Scope path can contain variable path segments as resources.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{App, HttpRequest, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().scope("/{project_id}", |scope| {
    ///         scope
    ///             .resource("/path1", |r| r.to(|| HttpResponse::Ok()))
    ///             .resource("/path2", |r| r.to(|| HttpResponse::Ok()))
    ///             .resource("/path3", |r| r.to(|| HttpResponse::MethodNotAllowed()))
    ///     });
    /// }
    /// ```
    ///
    /// In the above example, three routes get added:
    ///  * /{project_id}/path1
    ///  * /{project_id}/path2
    ///  * /{project_id}/path3
    ///
    pub fn scope<F>(self, path: &str, f: F) -> AppRouter<T, P, Body, AppEntry<P>>
    where
        F: FnOnce(Scope<P>) -> Scope<P>,
    {
        let mut scope = f(Scope::new(path));
        let rdef = scope.rdef().clone();
        let default = scope.get_default();
        let guards = scope.take_guards();

        let fref = Rc::new(RefCell::new(None));
        AppRouter {
            chain: self.chain,
            services: vec![(rdef, boxed::new_service(scope.into_new_service()), guards)],
            default: None,
            defaults: vec![default],
            endpoint: AppEntry::new(fref.clone()),
            factory_ref: fref,
            extensions: self.extensions,
            state: self.state,
            _t: PhantomData,
        }
    }

    /// Configure resource for a specific path.
    ///
    /// Resources may have variable path segments. For example, a
    /// resource with the path `/a/{name}/c` would match all incoming
    /// requests with paths such as `/a/b/c`, `/a/1/c`, or `/a/etc/c`.
    ///
    /// A variable segment is specified in the form `{identifier}`,
    /// where the identifier can be used later in a request handler to
    /// access the matched value for that segment. This is done by
    /// looking up the identifier in the `Params` object returned by
    /// `HttpRequest.match_info()` method.
    ///
    /// By default, each segment matches the regular expression `[^{}/]+`.
    ///
    /// You can also specify a custom regex in the form `{identifier:regex}`:
    ///
    /// For instance, to route `GET`-requests on any route matching
    /// `/users/{userid}/{friend}` and store `userid` and `friend` in
    /// the exposed `Params` object:
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{web, http, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().resource("/users/{userid}/{friend}", |r| {
    ///         r.route(web::get().to(|| HttpResponse::Ok()))
    ///          .route(web::head().to(|| HttpResponse::MethodNotAllowed()))
    ///     });
    /// }
    /// ```
    pub fn resource<F, U>(self, path: &str, f: F) -> AppRouter<T, P, Body, AppEntry<P>>
    where
        F: FnOnce(Resource<P>) -> Resource<P, U>,
        U: NewService<
                Request = ServiceRequest<P>,
                Response = ServiceResponse,
                Error = (),
                InitError = (),
            > + 'static,
    {
        let rdef = ResourceDef::new(&insert_slash(path));
        let res = f(Resource::new());
        let default = res.get_default();

        let fref = Rc::new(RefCell::new(None));
        AppRouter {
            chain: self.chain,
            services: vec![(rdef, boxed::new_service(res.into_new_service()), None)],
            default: None,
            defaults: vec![default],
            endpoint: AppEntry::new(fref.clone()),
            factory_ref: fref,
            extensions: self.extensions,
            state: self.state,
            _t: PhantomData,
        }
    }

    /// Register a middleware.
    pub fn middleware<M, B, F>(
        self,
        mw: F,
    ) -> AppRouter<
        T,
        P,
        B,
        impl NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B>,
            Error = (),
            InitError = (),
        >,
    >
    where
        M: Transform<
            AppRouting<P>,
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B>,
            Error = (),
            InitError = (),
        >,
        F: IntoTransform<M, AppRouting<P>>,
    {
        let fref = Rc::new(RefCell::new(None));
        let endpoint = ApplyTransform::new(mw, AppEntry::new(fref.clone()));
        AppRouter {
            endpoint,
            chain: self.chain,
            state: self.state,
            services: Vec::new(),
            default: None,
            defaults: Vec::new(),
            factory_ref: fref,
            extensions: self.extensions,
            _t: PhantomData,
        }
    }

    /// Register a request modifier. It can modify any request parameters
    /// including payload stream.
    pub fn chain<C, F, P1>(
        self,
        chain: C,
    ) -> App<
        P1,
        impl NewService<
            Request = ServiceRequest<PayloadStream>,
            Response = ServiceRequest<P1>,
            Error = (),
            InitError = (),
        >,
    >
    where
        C: NewService<
            (),
            Request = ServiceRequest<P>,
            Response = ServiceRequest<P1>,
            Error = (),
            InitError = (),
        >,
        F: IntoNewService<C>,
    {
        let chain = self.chain.and_then(chain.into_new_service());
        App {
            chain,
            state: self.state,
            extensions: self.extensions,
            _t: PhantomData,
        }
    }

    /// Set server host name.
    ///
    /// Host name is used by application router aa a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    ///
    /// By default host name is set to a "localhost" value.
    pub fn hostname(self, _val: &str) -> Self {
        // self.host = val.to_owned();
        self
    }
}

/// Application router builder - Structure that follows the builder pattern
/// for building application instances.
pub struct AppRouter<C, P, B, T> {
    chain: C,
    services: Vec<(ResourceDef, HttpNewService<P>, Option<Guards>)>,
    default: Option<Rc<HttpNewService<P>>>,
    defaults: Vec<Rc<RefCell<Option<Rc<HttpNewService<P>>>>>>,
    endpoint: T,
    factory_ref: Rc<RefCell<Option<AppRoutingFactory<P>>>>,
    extensions: Extensions,
    state: Vec<Box<StateFactory>>,
    _t: PhantomData<(P, B)>,
}

impl<C, P, B, T> AppRouter<C, P, B, T>
where
    P: 'static,
    B: MessageBody,
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse<B>,
        Error = (),
        InitError = (),
    >,
{
    /// Configure scope for common root path.
    ///
    /// Scopes collect multiple paths under a common path prefix.
    /// Scope path can contain variable path segments as resources.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{App, HttpRequest, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().scope("/{project_id}", |scope| {
    ///         scope
    ///             .resource("/path1", |r| r.to(|| HttpResponse::Ok()))
    ///             .resource("/path2", |r| r.to(|| HttpResponse::Ok()))
    ///             .resource("/path3", |r| r.to(|| HttpResponse::MethodNotAllowed()))
    ///     });
    /// }
    /// ```
    ///
    /// In the above example, three routes get added:
    ///  * /{project_id}/path1
    ///  * /{project_id}/path2
    ///  * /{project_id}/path3
    ///
    pub fn scope<F>(mut self, path: &str, f: F) -> Self
    where
        F: FnOnce(Scope<P>) -> Scope<P>,
    {
        let mut scope = f(Scope::new(path));
        let rdef = scope.rdef().clone();
        let guards = scope.take_guards();
        self.defaults.push(scope.get_default());
        self.services
            .push((rdef, boxed::new_service(scope.into_new_service()), guards));
        self
    }

    /// Configure resource for a specific path.
    ///
    /// Resources may have variable path segments. For example, a
    /// resource with the path `/a/{name}/c` would match all incoming
    /// requests with paths such as `/a/b/c`, `/a/1/c`, or `/a/etc/c`.
    ///
    /// A variable segment is specified in the form `{identifier}`,
    /// where the identifier can be used later in a request handler to
    /// access the matched value for that segment. This is done by
    /// looking up the identifier in the `Params` object returned by
    /// `HttpRequest.match_info()` method.
    ///
    /// By default, each segment matches the regular expression `[^{}/]+`.
    ///
    /// You can also specify a custom regex in the form `{identifier:regex}`:
    ///
    /// For instance, to route `GET`-requests on any route matching
    /// `/users/{userid}/{friend}` and store `userid` and `friend` in
    /// the exposed `Params` object:
    ///
    /// ```rust
    /// use actix_web::{web, http, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .resource("/users/{userid}/{friend}", |r| {
    ///             r.route(web::to(|| HttpResponse::Ok()))
    ///         })
    ///         .resource("/index.html", |r| {
    ///             r.route(web::head().to(|| HttpResponse::MethodNotAllowed()))
    ///         });
    /// }
    /// ```
    pub fn resource<F, U>(mut self, path: &str, f: F) -> Self
    where
        F: FnOnce(Resource<P>) -> Resource<P, U>,
        U: NewService<
                Request = ServiceRequest<P>,
                Response = ServiceResponse,
                Error = (),
                InitError = (),
            > + 'static,
    {
        let rdef = ResourceDef::new(&insert_slash(path));
        let resource = f(Resource::new());
        self.defaults.push(resource.get_default());
        self.services.push((
            rdef,
            boxed::new_service(resource.into_new_service()),
            None,
        ));
        self
    }

    /// Default resource to be used if no matching route could be found.
    ///
    /// Default resource works with resources only and does not work with
    /// custom services.
    pub fn default_resource<F, R, U>(mut self, f: F) -> Self
    where
        F: FnOnce(Resource<P>) -> R,
        R: IntoNewService<U>,
        U: NewService<
                Request = ServiceRequest<P>,
                Response = ServiceResponse,
                Error = (),
            > + 'static,
    {
        // create and configure default resource
        self.default = Some(Rc::new(boxed::new_service(
            f(Resource::new()).into_new_service().map_init_err(|_| ()),
        )));

        self
    }

    /// Register resource handler service.
    pub fn service<R, F, U>(mut self, rdef: R, factory: F) -> Self
    where
        R: Into<ResourceDef>,
        F: IntoNewService<U>,
        U: NewService<
                Request = ServiceRequest<P>,
                Response = ServiceResponse,
                Error = (),
            > + 'static,
    {
        self.services.push((
            rdef.into(),
            boxed::new_service(factory.into_new_service().map_init_err(|_| ())),
            None,
        ));
        self
    }

    /// Register a middleware.
    pub fn middleware<M, B1, F>(
        self,
        mw: F,
    ) -> AppRouter<
        C,
        P,
        B1,
        impl NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B1>,
            Error = (),
            InitError = (),
        >,
    >
    where
        M: Transform<
            T::Service,
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B1>,
            Error = (),
            InitError = (),
        >,
        B1: MessageBody,
        F: IntoTransform<M, T::Service>,
    {
        let endpoint = ApplyTransform::new(mw, self.endpoint);
        AppRouter {
            endpoint,
            chain: self.chain,
            state: self.state,
            services: self.services,
            default: self.default,
            defaults: self.defaults,
            factory_ref: self.factory_ref,
            extensions: self.extensions,
            _t: PhantomData,
        }
    }

    /// Register an external resource.
    ///
    /// External resources are useful for URL generation purposes only
    /// and are never considered for matching at request time. Calls to
    /// `HttpRequest::url_for()` will work as expected.
    ///
    /// ```rust,ignore
    /// # extern crate actix_web;
    /// use actix_web::{App, HttpRequest, HttpResponse, Result};
    ///
    /// fn index(req: &HttpRequest) -> Result<HttpResponse> {
    ///     let url = req.url_for("youtube", &["oHg5SJYRHA0"])?;
    ///     assert_eq!(url.as_str(), "https://youtube.com/watch/oHg5SJYRHA0");
    ///     Ok(HttpResponse::Ok().into())
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .resource("/index.html", |r| r.get().f(index))
    ///         .external_resource("youtube", "https://youtube.com/watch/{video_id}")
    ///         .finish();
    /// }
    /// ```
    pub fn external_resource<N, U>(self, _name: N, _url: U) -> Self
    where
        N: AsRef<str>,
        U: AsRef<str>,
    {
        // self.parts
        //     .as_mut()
        //     .expect("Use after finish")
        //     .router
        //     .register_external(name.as_ref(), ResourceDef::external(url.as_ref()));
        self
    }
}

impl<C, T, P: 'static, B: MessageBody>
    IntoNewService<AndThenNewService<AppInit<C, P>, T, ()>> for AppRouter<C, P, B, T>
where
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse<B>,
        Error = (),
        InitError = (),
    >,
    C: NewService<
        Request = ServiceRequest<PayloadStream>,
        Response = ServiceRequest<P>,
        Error = (),
        InitError = (),
    >,
{
    fn into_new_service(self) -> AndThenNewService<AppInit<C, P>, T, ()> {
        // update resource default service
        if self.default.is_some() {
            for default in &self.defaults {
                if default.borrow_mut().is_none() {
                    *default.borrow_mut() = self.default.clone();
                }
            }
        }

        // set factory
        *self.factory_ref.borrow_mut() = Some(AppRoutingFactory {
            default: self.default.clone(),
            services: Rc::new(
                self.services
                    .into_iter()
                    .map(|(rdef, srv, guards)| (rdef, srv, RefCell::new(guards)))
                    .collect(),
            ),
        });

        AppInit {
            chain: self.chain,
            state: self.state,
            extensions: Rc::new(RefCell::new(Rc::new(self.extensions))),
        }
        .and_then(self.endpoint)
    }
}

pub struct AppRoutingFactory<P> {
    services: Rc<Vec<(ResourceDef, HttpNewService<P>, RefCell<Option<Guards>>)>>,
    default: Option<Rc<HttpNewService<P>>>,
}

impl<P: 'static> NewService for AppRoutingFactory<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = AppRouting<P>;
    type Future = AppRoutingFactoryResponse<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        let default_fut = if let Some(ref default) = self.default {
            Some(default.new_service(&()))
        } else {
            None
        };

        AppRoutingFactoryResponse {
            fut: self
                .services
                .iter()
                .map(|(path, service, guards)| {
                    CreateAppRoutingItem::Future(
                        Some(path.clone()),
                        guards.borrow_mut().take(),
                        service.new_service(&()),
                    )
                })
                .collect(),
            default: None,
            default_fut,
        }
    }
}

type HttpServiceFut<P> = Box<Future<Item = HttpService<P>, Error = ()>>;

/// Create app service
#[doc(hidden)]
pub struct AppRoutingFactoryResponse<P> {
    fut: Vec<CreateAppRoutingItem<P>>,
    default: Option<HttpService<P>>,
    default_fut: Option<Box<Future<Item = HttpService<P>, Error = ()>>>,
}

enum CreateAppRoutingItem<P> {
    Future(Option<ResourceDef>, Option<Guards>, HttpServiceFut<P>),
    Service(ResourceDef, Option<Guards>, HttpService<P>),
}

impl<P> Future for AppRoutingFactoryResponse<P> {
    type Item = AppRouting<P>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut done = true;

        if let Some(ref mut fut) = self.default_fut {
            match fut.poll()? {
                Async::Ready(default) => self.default = Some(default),
                Async::NotReady => done = false,
            }
        }

        // poll http services
        for item in &mut self.fut {
            let res = match item {
                CreateAppRoutingItem::Future(
                    ref mut path,
                    ref mut guards,
                    ref mut fut,
                ) => match fut.poll()? {
                    Async::Ready(service) => {
                        Some((path.take().unwrap(), guards.take(), service))
                    }
                    Async::NotReady => {
                        done = false;
                        None
                    }
                },
                CreateAppRoutingItem::Service(_, _, _) => continue,
            };

            if let Some((path, guards, service)) = res {
                *item = CreateAppRoutingItem::Service(path, guards, service);
            }
        }

        if done {
            let router = self
                .fut
                .drain(..)
                .fold(Router::build(), |mut router, item| {
                    match item {
                        CreateAppRoutingItem::Service(path, guards, service) => {
                            router.rdef(path, service);
                            router.set_user_data(guards);
                        }
                        CreateAppRoutingItem::Future(_, _, _) => unreachable!(),
                    }
                    router
                });
            Ok(Async::Ready(AppRouting {
                ready: None,
                router: router.finish(),
                default: self.default.take(),
            }))
        } else {
            Ok(Async::NotReady)
        }
    }
}

pub struct AppRouting<P> {
    router: Router<HttpService<P>, Guards>,
    ready: Option<(ServiceRequest<P>, ResourceInfo)>,
    default: Option<HttpService<P>>,
}

impl<P> Service for AppRouting<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type Future = Either<BoxedResponse, FutureResult<Self::Response, Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        if self.ready.is_none() {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }

    fn call(&mut self, mut req: ServiceRequest<P>) -> Self::Future {
        let res = self.router.recognize_mut_checked(&mut req, |req, guards| {
            if let Some(ref guards) = guards {
                for f in guards {
                    if !f.check(req.head()) {
                        return false;
                    }
                }
            }
            true
        });

        if let Some((srv, _info)) = res {
            Either::A(srv.call(req))
        } else if let Some(ref mut default) = self.default {
            Either::A(default.call(req))
        } else {
            let req = req.into_request();
            Either::B(ok(ServiceResponse::new(req, Response::NotFound().finish())))
        }
    }
}

#[doc(hidden)]
/// Wrapper service for routing
pub struct AppEntry<P> {
    factory: Rc<RefCell<Option<AppRoutingFactory<P>>>>,
}

impl<P> AppEntry<P> {
    fn new(factory: Rc<RefCell<Option<AppRoutingFactory<P>>>>) -> Self {
        AppEntry { factory }
    }
}

impl<P: 'static> NewService for AppEntry<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = AppRouting<P>;
    type Future = AppRoutingFactoryResponse<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        self.factory.borrow_mut().as_mut().unwrap().new_service(&())
    }
}

#[doc(hidden)]
pub struct AppChain;

impl NewService<()> for AppChain {
    type Request = ServiceRequest<PayloadStream>;
    type Response = ServiceRequest<PayloadStream>;
    type Error = ();
    type InitError = ();
    type Service = AppChain;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(AppChain)
    }
}

impl Service for AppChain {
    type Request = ServiceRequest<PayloadStream>;
    type Response = ServiceRequest<PayloadStream>;
    type Error = ();
    type Future = FutureResult<Self::Response, Self::Error>;

    #[inline]
    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    #[inline]
    fn call(&mut self, req: Self::Request) -> Self::Future {
        ok(req)
    }
}

/// Service factory to convert `Request` to a `ServiceRequest<S>`.
/// It also executes state factories.
pub struct AppInit<C, P>
where
    C: NewService<Request = ServiceRequest<PayloadStream>, Response = ServiceRequest<P>>,
{
    chain: C,
    state: Vec<Box<StateFactory>>,
    extensions: Rc<RefCell<Rc<Extensions>>>,
}

impl<C, P: 'static> NewService for AppInit<C, P>
where
    C: NewService<
        Request = ServiceRequest<PayloadStream>,
        Response = ServiceRequest<P>,
        InitError = (),
    >,
{
    type Request = Request<PayloadStream>;
    type Response = ServiceRequest<P>;
    type Error = C::Error;
    type InitError = C::InitError;
    type Service = AppInitService<C::Service, P>;
    type Future = AppInitResult<C, P>;

    fn new_service(&self, _: &()) -> Self::Future {
        AppInitResult {
            chain: self.chain.new_service(&()),
            state: self.state.iter().map(|s| s.construct()).collect(),
            extensions: self.extensions.clone(),
        }
    }
}

#[doc(hidden)]
pub struct AppInitResult<C, P>
where
    C: NewService<
        Request = ServiceRequest<PayloadStream>,
        Response = ServiceRequest<P>,
        InitError = (),
    >,
{
    chain: C::Future,
    state: Vec<Box<StateFactoryResult>>,
    extensions: Rc<RefCell<Rc<Extensions>>>,
}

impl<C, P> Future for AppInitResult<C, P>
where
    C: NewService<
        Request = ServiceRequest<PayloadStream>,
        Response = ServiceRequest<P>,
        InitError = (),
    >,
{
    type Item = AppInitService<C::Service, P>;
    type Error = C::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(extensions) = Rc::get_mut(&mut *self.extensions.borrow_mut()) {
            let mut idx = 0;
            while idx < self.state.len() {
                if let Async::Ready(_) = self.state[idx].poll_result(extensions)? {
                    self.state.remove(idx);
                } else {
                    idx += 1;
                }
            }
            if !self.state.is_empty() {
                return Ok(Async::NotReady);
            }
        } else {
            log::warn!("Multiple copies of app extensions exists");
        }

        let chain = futures::try_ready!(self.chain.poll());

        Ok(Async::Ready(AppInitService {
            chain,
            extensions: self.extensions.borrow().clone(),
        }))
    }
}

/// Service to convert `Request` to a `ServiceRequest<S>`
pub struct AppInitService<C, P>
where
    C: Service<Request = ServiceRequest<PayloadStream>, Response = ServiceRequest<P>>,
{
    chain: C,
    extensions: Rc<Extensions>,
}

impl<C, P> Service for AppInitService<C, P>
where
    C: Service<Request = ServiceRequest<PayloadStream>, Response = ServiceRequest<P>>,
{
    type Request = Request<PayloadStream>;
    type Response = ServiceRequest<P>;
    type Error = C::Error;
    type Future = C::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.chain.poll_ready()
    }

    fn call(&mut self, req: Request<PayloadStream>) -> Self::Future {
        let req = ServiceRequest::new(
            Path::new(Url::new(req.uri().clone())),
            req,
            self.extensions.clone(),
        );
        self.chain.call(req)
    }
}

#[cfg(test)]
mod tests {
    use actix_http::http::{Method, StatusCode};

    use super::*;
    use crate::test::{block_on, TestRequest};
    use crate::{web, HttpResponse, State};

    #[test]
    fn test_default_resource() {
        let app = App::new()
            .resource("/test", |r| r.to(|| HttpResponse::Ok()))
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/test").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/blah").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let app = App::new()
            .resource("/test", |r| r.to(|| HttpResponse::Ok()))
            .resource("/test2", |r| {
                r.default_resource(|r| r.to(|| HttpResponse::Created()))
                    .route(web::get().to(|| HttpResponse::Ok()))
            })
            .default_resource(|r| r.to(|| HttpResponse::MethodNotAllowed()))
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/blah").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/test2").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test2")
            .method(Method::POST)
            .to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_state() {
        let app = App::new()
            .state(10usize)
            .resource("/", |r| r.to(|_: State<usize>| HttpResponse::Ok()))
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app = App::new()
            .state(10u32)
            .resource("/", |r| r.to(|_: State<usize>| HttpResponse::Ok()))
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_state_factory() {
        let app = App::new()
            .state_factory(|| Ok::<_, ()>(10usize))
            .resource("/", |r| r.to(|_: State<usize>| HttpResponse::Ok()))
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app = App::new()
            .state_factory(|| Ok::<_, ()>(10u32))
            .resource("/", |r| r.to(|_: State<usize>| HttpResponse::Ok()))
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // #[test]
    // fn test_handler() {
    //     let app = App::new()
    //         .handler("/test", |_: &_| HttpResponse::Ok())
    //         .finish();

    //     let req = TestRequest::with_uri("/test").request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::OK);

    //     let req = TestRequest::with_uri("/test/").request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::OK);

    //     let req = TestRequest::with_uri("/test/app").request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::OK);

    //     let req = TestRequest::with_uri("/testapp").request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

    //     let req = TestRequest::with_uri("/blah").request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    // }

    // #[test]
    // fn test_handler2() {
    //     let app = App::new()
    //         .handler("test", |_: &_| HttpResponse::Ok())
    //         .finish();

    //     let req = TestRequest::with_uri("/test").request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::OK);

    //     let req = TestRequest::with_uri("/test/").request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::OK);

    //     let req = TestRequest::with_uri("/test/app").request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::OK);

    //     let req = TestRequest::with_uri("/testapp").request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);

    //     let req = TestRequest::with_uri("/blah").request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    // }

    // #[test]
    // fn test_route() {
    //     let app = App::new()
    //         .route("/test", Method::GET, |_: HttpRequest| HttpResponse::Ok())
    //         .route("/test", Method::POST, |_: HttpRequest| {
    //             HttpResponse::Created()
    //         })
    //         .finish();

    //     let req = TestRequest::with_uri("/test").method(Method::GET).request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::OK);

    //     let req = TestRequest::with_uri("/test")
    //         .method(Method::POST)
    //         .request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::CREATED);

    //     let req = TestRequest::with_uri("/test")
    //         .method(Method::HEAD)
    //         .request();
    //     let resp = app.run(req);
    //     assert_eq!(resp.as_msg().status(), StatusCode::NOT_FOUND);
    // }
}
