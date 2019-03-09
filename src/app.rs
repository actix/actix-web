use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::body::{Body, MessageBody};
use actix_http::{Extensions, PayloadStream, Request, Response};
use actix_router::{Path, ResourceDef, ResourceInfo, Router, Url};
use actix_service::boxed::{self, BoxedNewService, BoxedService};
use actix_service::{
    fn_service, AndThenNewService, ApplyTransform, IntoNewService, IntoTransform,
    NewService, Service, Transform,
};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, IntoFuture, Poll};

use crate::config::AppConfig;
use crate::guard::Guard;
use crate::resource::Resource;
use crate::rmap::ResourceMap;
use crate::route::Route;
use crate::service::{
    HttpServiceFactory, ServiceFactory, ServiceFactoryWrapper, ServiceRequest,
    ServiceResponse,
};
use crate::state::{State, StateFactory, StateFactoryResult};

type Guards = Vec<Box<Guard>>;
type HttpService<P> = BoxedService<ServiceRequest<P>, ServiceResponse, ()>;
type HttpNewService<P> = BoxedNewService<(), ServiceRequest<P>, ServiceResponse, (), ()>;
type BoxedResponse = Box<Future<Item = ServiceResponse, Error = ()>>;

/// Application builder - structure that follows the builder pattern
/// for building application instances.
pub struct App<P, T>
where
    T: NewService<ServiceRequest, Response = ServiceRequest<P>>,
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
        ServiceRequest,
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
    /// use actix_web::{web, App};
    ///
    /// struct MyState {
    ///     counter: Cell<usize>,
    /// }
    ///
    /// fn index(state: web::State<MyState>) {
    ///     state.counter.set(state.counter.get() + 1);
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .state(MyState{ counter: Cell::new(0) })
    ///         .service(
    ///             web::resource("/index.html").route(
    ///                 web::get().to(index)));
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

    /// Register a middleware.
    pub fn middleware<M, B, F>(
        self,
        mw: F,
    ) -> AppRouter<
        T,
        P,
        B,
        impl NewService<
            ServiceRequest<P>,
            Response = ServiceResponse<B>,
            Error = (),
            InitError = (),
        >,
    >
    where
        M: Transform<
            AppRouting<P>,
            ServiceRequest<P>,
            Response = ServiceResponse<B>,
            Error = (),
            InitError = (),
        >,
        F: IntoTransform<M, AppRouting<P>, ServiceRequest<P>>,
    {
        let fref = Rc::new(RefCell::new(None));
        let endpoint = ApplyTransform::new(mw, AppEntry::new(fref.clone()));
        AppRouter {
            endpoint,
            chain: self.chain,
            state: self.state,
            services: Vec::new(),
            default: None,
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
            ServiceRequest,
            Response = ServiceRequest<P1>,
            Error = (),
            InitError = (),
        >,
    >
    where
        C: NewService<
            ServiceRequest<P>,
            Response = ServiceRequest<P1>,
            Error = (),
            InitError = (),
        >,
        F: IntoNewService<C, ServiceRequest<P>>,
    {
        let chain = self.chain.and_then(chain.into_new_service());
        App {
            chain,
            state: self.state,
            extensions: self.extensions,
            _t: PhantomData,
        }
    }

    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `App::service()` method.
    /// This method can not be could multiple times, in that case
    /// multiple resources with one route would be registered for same resource path.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// fn index(data: web::Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .route("/test1", web::get().to(index))
    ///         .route("/test2", web::post().to(|| HttpResponse::MethodNotAllowed()));
    /// }
    /// ```
    pub fn route(
        self,
        path: &str,
        mut route: Route<P>,
    ) -> AppRouter<T, P, Body, AppEntry<P>> {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Register http service.
    pub fn service<F>(self, service: F) -> AppRouter<T, P, Body, AppEntry<P>>
    where
        F: HttpServiceFactory<P> + 'static,
    {
        let fref = Rc::new(RefCell::new(None));

        AppRouter {
            chain: self.chain,
            default: None,
            endpoint: AppEntry::new(fref.clone()),
            factory_ref: fref,
            extensions: self.extensions,
            state: self.state,
            services: vec![Box::new(ServiceFactoryWrapper::new(service))],
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
    endpoint: T,
    services: Vec<Box<ServiceFactory<P>>>,
    default: Option<Rc<HttpNewService<P>>>,
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
        ServiceRequest<P>,
        Response = ServiceResponse<B>,
        Error = (),
        InitError = (),
    >,
{
    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `App::service()` method.
    /// This method can not be could multiple times, in that case
    /// multiple resources with one route would be registered for same resource path.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// fn index(data: web::Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .route("/test1", web::get().to(index))
    ///         .route("/test2", web::post().to(|| HttpResponse::MethodNotAllowed()));
    /// }
    /// ```
    pub fn route(self, path: &str, mut route: Route<P>) -> Self {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Register http service.
    ///
    /// Http service is any type that implements `HttpServiceFactory` trait.
    ///
    /// Actix web provides several services implementations:
    ///
    /// * *Resource* is an entry in route table which corresponds to requested URL.
    /// * *Scope* is a set of resources with common root path.
    /// * "StaticFiles" is a service for static files support
    pub fn service<F>(mut self, factory: F) -> Self
    where
        F: HttpServiceFactory<P> + 'static,
    {
        self.services
            .push(Box::new(ServiceFactoryWrapper::new(factory)));
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
            ServiceRequest<P>,
            Response = ServiceResponse<B1>,
            Error = (),
            InitError = (),
        >,
    >
    where
        M: Transform<
            T::Service,
            ServiceRequest<P>,
            Response = ServiceResponse<B1>,
            Error = (),
            InitError = (),
        >,
        B1: MessageBody,
        F: IntoTransform<M, T::Service, ServiceRequest<P>>,
    {
        let endpoint = ApplyTransform::new(mw, self.endpoint);
        AppRouter {
            endpoint,
            chain: self.chain,
            state: self.state,
            services: self.services,
            default: self.default,
            factory_ref: self.factory_ref,
            extensions: self.extensions,
            _t: PhantomData,
        }
    }

    /// Default resource to be used if no matching route could be found.
    ///
    /// Default resource works with resources only and does not work with
    /// custom services.
    pub fn default_resource<F, U>(mut self, f: F) -> Self
    where
        F: FnOnce(Resource<P>) -> Resource<P, U>,
        U: NewService<
                ServiceRequest<P>,
                Response = ServiceResponse,
                Error = (),
                InitError = (),
            > + 'static,
    {
        // create and configure default resource
        self.default = Some(Rc::new(boxed::new_service(
            f(Resource::new("")).into_new_service().map_init_err(|_| ()),
        )));

        self
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
    IntoNewService<AndThenNewService<AppInit<C, P>, T>, Request>
    for AppRouter<C, P, B, T>
where
    T: NewService<
        ServiceRequest<P>,
        Response = ServiceResponse<B>,
        Error = (),
        InitError = (),
    >,
    C: NewService<
        ServiceRequest,
        Response = ServiceRequest<P>,
        Error = (),
        InitError = (),
    >,
{
    fn into_new_service(self) -> AndThenNewService<AppInit<C, P>, T> {
        // update resource default service
        let default = self.default.unwrap_or_else(|| {
            Rc::new(boxed::new_service(fn_service(|req: ServiceRequest<P>| {
                Ok(req.into_response(Response::NotFound().finish()))
            })))
        });

        let mut config = AppConfig::new(
            "127.0.0.1:8080".parse().unwrap(),
            "localhost:8080".to_owned(),
            false,
            default.clone(),
        );

        // register services
        self.services
            .into_iter()
            .for_each(|mut srv| srv.register(&mut config));

        let mut rmap = ResourceMap::new(ResourceDef::new(""));

        // complete pipeline creation
        *self.factory_ref.borrow_mut() = Some(AppRoutingFactory {
            default,
            services: Rc::new(
                config
                    .into_services()
                    .into_iter()
                    .map(|(mut rdef, srv, guards, nested)| {
                        rmap.add(&mut rdef, nested);
                        (rdef, srv, RefCell::new(guards))
                    })
                    .collect(),
            ),
        });

        // complete ResourceMap tree creation
        let rmap = Rc::new(rmap);
        rmap.finish(rmap.clone());

        AppInit {
            rmap,
            chain: self.chain,
            state: self.state,
            extensions: Rc::new(RefCell::new(Rc::new(self.extensions))),
        }
        .and_then(self.endpoint)
    }
}

pub struct AppRoutingFactory<P> {
    services: Rc<Vec<(ResourceDef, HttpNewService<P>, RefCell<Option<Guards>>)>>,
    default: Rc<HttpNewService<P>>,
}

impl<P: 'static> NewService<ServiceRequest<P>> for AppRoutingFactory<P> {
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = AppRouting<P>;
    type Future = AppRoutingFactoryResponse<P>;

    fn new_service(&self, _: &()) -> Self::Future {
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
            default_fut: Some(self.default.new_service(&())),
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
                            router.rdef(path, service).2 = guards;
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

impl<P> Service<ServiceRequest<P>> for AppRouting<P> {
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

impl<P: 'static> NewService<ServiceRequest<P>> for AppEntry<P> {
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

impl NewService<ServiceRequest> for AppChain {
    type Response = ServiceRequest;
    type Error = ();
    type InitError = ();
    type Service = AppChain;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(AppChain)
    }
}

impl Service<ServiceRequest> for AppChain {
    type Response = ServiceRequest;
    type Error = ();
    type Future = FutureResult<Self::Response, Self::Error>;

    #[inline]
    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    #[inline]
    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        ok(req)
    }
}

/// Service factory to convert `Request` to a `ServiceRequest<S>`.
/// It also executes state factories.
pub struct AppInit<C, P>
where
    C: NewService<ServiceRequest, Response = ServiceRequest<P>>,
{
    chain: C,
    rmap: Rc<ResourceMap>,
    state: Vec<Box<StateFactory>>,
    extensions: Rc<RefCell<Rc<Extensions>>>,
}

impl<C, P: 'static> NewService<Request> for AppInit<C, P>
where
    C: NewService<ServiceRequest, Response = ServiceRequest<P>, InitError = ()>,
{
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
            rmap: self.rmap.clone(),
        }
    }
}

#[doc(hidden)]
pub struct AppInitResult<C, P>
where
    C: NewService<ServiceRequest, Response = ServiceRequest<P>, InitError = ()>,
{
    chain: C::Future,
    rmap: Rc<ResourceMap>,
    state: Vec<Box<StateFactoryResult>>,
    extensions: Rc<RefCell<Rc<Extensions>>>,
}

impl<C, P> Future for AppInitResult<C, P>
where
    C: NewService<ServiceRequest, Response = ServiceRequest<P>, InitError = ()>,
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
            rmap: self.rmap.clone(),
            extensions: self.extensions.borrow().clone(),
        }))
    }
}

/// Service to convert `Request` to a `ServiceRequest<S>`
pub struct AppInitService<C, P>
where
    C: Service<ServiceRequest, Response = ServiceRequest<P>>,
{
    chain: C,
    rmap: Rc<ResourceMap>,
    extensions: Rc<Extensions>,
}

impl<C, P> Service<Request> for AppInitService<C, P>
where
    C: Service<ServiceRequest, Response = ServiceRequest<P>>,
{
    type Response = ServiceRequest<P>;
    type Error = C::Error;
    type Future = C::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.chain.poll_ready()
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let req = ServiceRequest::new(
            Path::new(Url::new(req.uri().clone())),
            req,
            self.rmap.clone(),
            self.extensions.clone(),
        );
        self.chain.call(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Method, StatusCode};
    use crate::test::{block_on, init_service, TestRequest};
    use crate::{web, HttpResponse};

    #[test]
    fn test_default_resource() {
        let mut srv = init_service(
            App::new().service(web::resource("/test").to(|| HttpResponse::Ok())),
        );
        let req = TestRequest::with_uri("/test").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/blah").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut srv = init_service(
            App::new()
                .service(web::resource("/test").to(|| HttpResponse::Ok()))
                .service(
                    web::resource("/test2")
                        .default_resource(|r| r.to(|| HttpResponse::Created()))
                        .route(web::get().to(|| HttpResponse::Ok())),
                )
                .default_resource(|r| r.to(|| HttpResponse::MethodNotAllowed())),
        );

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
        let mut srv =
            init_service(App::new().state(10usize).service(
                web::resource("/").to(|_: web::State<usize>| HttpResponse::Ok()),
            ));

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().state(10u32).service(
                web::resource("/").to(|_: web::State<usize>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_state_factory() {
        let mut srv =
            init_service(App::new().state_factory(|| Ok::<_, ()>(10usize)).service(
                web::resource("/").to(|_: web::State<usize>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().state_factory(|| Ok::<_, ()>(10u32)).service(
                web::resource("/").to(|_: web::State<usize>| HttpResponse::Ok()),
            ));
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
