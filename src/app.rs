use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::body::{Body, MessageBody};
use actix_http::{Extensions, PayloadStream, Request, Response};
use actix_router::{Path, ResourceDef, ResourceInfo, Router, Url};
use actix_service::boxed::{self, BoxedNewService, BoxedService};
use actix_service::{
    AndThenNewService, ApplyNewService, IntoNewService, IntoNewTransform, NewService,
    NewTransform, Service,
};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, IntoFuture, Poll};

use crate::resource::Resource;
use crate::service::{ServiceRequest, ServiceResponse};
use crate::state::{State, StateFactory, StateFactoryResult};

type HttpService<P> = BoxedService<ServiceRequest<P>, ServiceResponse, ()>;
type HttpNewService<P> = BoxedNewService<(), ServiceRequest<P>, ServiceResponse, (), ()>;
type BoxedResponse = Box<Future<Item = ServiceResponse, Error = ()>>;

pub trait HttpServiceFactory<Request> {
    type Factory: NewService<Request = Request>;

    fn rdef(&self) -> &ResourceDef;

    fn create(self) -> Self::Factory;
}

/// Application builder
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

impl Default for App<PayloadStream, AppChain> {
    fn default() -> Self {
        App::new()
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
    /// Create application with specified state. Application can be
    /// configured with a builder-like pattern.
    ///
    /// State is shared with all resources within same application and
    /// could be accessed with `HttpRequest::state()` method.
    ///
    /// **Note**: http server accepts an application factory rather than
    /// an application instance. Http server constructs an application
    /// instance for each thread, thus application state must be constructed
    /// multiple times. If you want to share state between different
    /// threads, a shared object should be used, e.g. `Arc`. Application
    /// state does not need to be `Send` or `Sync`.
    pub fn state<S: 'static>(mut self, state: S) -> Self {
        self.state.push(Box::new(State::new(state)));
        self
    }

    /// Set application state. This function is
    /// similar to `.state()` but it accepts state factory. State get
    /// constructed asynchronously during application initialization.
    pub fn state_factory<S, F, Out>(mut self, state: F) -> Self
    where
        F: Fn() -> Out + 'static,
        Out: IntoFuture + 'static,
        Out::Error: std::fmt::Debug,
    {
        self.state.push(Box::new(State::new(state)));
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
    /// ```rust,ignore
    /// # extern crate actix_web;
    /// use actix_web::{http, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().resource("/users/{userid}/{friend}", |r| {
    ///         r.get(|r| r.to(|_| HttpResponse::Ok()));
    ///         r.head(|r| r.to(|_| HttpResponse::MethodNotAllowed()))
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
        let rdef = ResourceDef::new(path);
        let resource = f(Resource::new());
        let default = resource.get_default();

        let fref = Rc::new(RefCell::new(None));
        AppRouter {
            chain: self.chain,
            services: vec![(rdef, boxed::new_service(resource.into_new_service()))],
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
    pub fn middleware<M, F>(
        self,
        mw: F,
    ) -> AppRouter<
        T,
        P,
        Body,
        impl NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse,
            Error = (),
            InitError = (),
        >,
    >
    where
        M: NewTransform<
            AppService<P>,
            Request = ServiceRequest<P>,
            Response = ServiceResponse,
            Error = (),
            InitError = (),
        >,
        F: IntoNewTransform<M, AppService<P>>,
    {
        let fref = Rc::new(RefCell::new(None));
        let endpoint = ApplyNewService::new(mw, AppEntry::new(fref.clone()));
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

    /// Complete applicatin chain configuration and start resource
    /// configuration.
    pub fn router<B>(self) -> AppRouter<T, P, B, AppEntry<P>> {
        let fref = Rc::new(RefCell::new(None));
        AppRouter {
            chain: self.chain,
            services: Vec::new(),
            default: None,
            defaults: Vec::new(),
            endpoint: AppEntry::new(fref.clone()),
            factory_ref: fref,
            extensions: self.extensions,
            state: self.state,
            _t: PhantomData,
        }
    }
}

/// Structure that follows the builder pattern for building application
/// instances.
pub struct AppRouter<C, P, B, T> {
    chain: C,
    services: Vec<(ResourceDef, HttpNewService<P>)>,
    default: Option<Rc<HttpNewService<P>>>,
    defaults: Vec<Rc<RefCell<Option<Rc<HttpNewService<P>>>>>>,
    endpoint: T,
    factory_ref: Rc<RefCell<Option<AppFactory<P>>>>,
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
    /// ```rust,ignore
    /// # extern crate actix_web;
    /// use actix_web::{http, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().resource("/users/{userid}/{friend}", |r| {
    ///         r.get(|r| r.to(|_| HttpResponse::Ok()));
    ///         r.head(|r| r.to(|_| HttpResponse::MethodNotAllowed()))
    ///     });
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
        let rdef = ResourceDef::new(path);
        let resource = f(Resource::new());
        self.defaults.push(resource.get_default());
        self.services
            .push((rdef, boxed::new_service(resource.into_new_service())));
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
        M: NewTransform<
            T::Service,
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B1>,
            Error = (),
            InitError = (),
        >,
        B1: MessageBody,
        F: IntoNewTransform<M, T::Service>,
    {
        let endpoint = ApplyNewService::new(mw, self.endpoint);
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
        *self.factory_ref.borrow_mut() = Some(AppFactory {
            services: Rc::new(self.services),
        });

        AppInit {
            chain: self.chain,
            state: self.state,
            extensions: Rc::new(RefCell::new(Rc::new(self.extensions))),
        }
        .and_then(self.endpoint)
    }
}

pub struct AppFactory<P> {
    services: Rc<Vec<(ResourceDef, HttpNewService<P>)>>,
}

impl<P: 'static> NewService for AppFactory<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = AppService<P>;
    type Future = CreateAppService<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        CreateAppService {
            fut: self
                .services
                .iter()
                .map(|(path, service)| {
                    CreateAppServiceItem::Future(
                        Some(path.clone()),
                        service.new_service(&()),
                    )
                })
                .collect(),
        }
    }
}

type HttpServiceFut<P> = Box<Future<Item = HttpService<P>, Error = ()>>;

/// Create app service
#[doc(hidden)]
pub struct CreateAppService<P> {
    fut: Vec<CreateAppServiceItem<P>>,
}

enum CreateAppServiceItem<P> {
    Future(Option<ResourceDef>, HttpServiceFut<P>),
    Service(ResourceDef, HttpService<P>),
}

impl<P> Future for CreateAppService<P> {
    type Item = AppService<P>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut done = true;

        // poll http services
        for item in &mut self.fut {
            let res = match item {
                CreateAppServiceItem::Future(ref mut path, ref mut fut) => {
                    match fut.poll()? {
                        Async::Ready(service) => Some((path.take().unwrap(), service)),
                        Async::NotReady => {
                            done = false;
                            None
                        }
                    }
                }
                CreateAppServiceItem::Service(_, _) => continue,
            };

            if let Some((path, service)) = res {
                *item = CreateAppServiceItem::Service(path, service);
            }
        }

        if done {
            let router = self
                .fut
                .drain(..)
                .fold(Router::build(), |mut router, item| {
                    match item {
                        CreateAppServiceItem::Service(path, service) => {
                            router.rdef(path, service)
                        }
                        CreateAppServiceItem::Future(_, _) => unreachable!(),
                    }
                    router
                });
            Ok(Async::Ready(AppService {
                router: router.finish(),
                ready: None,
            }))
        } else {
            Ok(Async::NotReady)
        }
    }
}

pub struct AppService<P> {
    router: Router<HttpService<P>>,
    ready: Option<(ServiceRequest<P>, ResourceInfo)>,
}

impl<P> Service for AppService<P> {
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
        if let Some((srv, _info)) = self.router.recognize_mut(req.match_info_mut()) {
            Either::A(srv.call(req))
        } else {
            let req = req.into_request();
            Either::B(ok(ServiceResponse::new(req, Response::NotFound().finish())))
        }
    }
}

#[doc(hidden)]
pub struct AppEntry<P> {
    factory: Rc<RefCell<Option<AppFactory<P>>>>,
}

impl<P> AppEntry<P> {
    fn new(factory: Rc<RefCell<Option<AppFactory<P>>>>) -> Self {
        AppEntry { factory }
    }
}

impl<P: 'static> NewService for AppEntry<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = AppService<P>;
    type Future = CreateAppService<P>;

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

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        ok(req)
    }
}

/// Service factory to convert `Request` to a `ServiceRequest<S>`
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
