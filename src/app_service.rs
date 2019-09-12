use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::{Extensions, Request, Response};
use actix_router::{Path, ResourceDef, ResourceInfo, Router, Url};
use actix_server_config::ServerConfig;
use actix_service::boxed::{self, BoxedNewService, BoxedService};
use actix_service::{service_fn, NewService, Service};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, Poll};

use crate::config::{AppConfig, AppService};
use crate::data::DataFactory;
use crate::error::Error;
use crate::guard::Guard;
use crate::request::{HttpRequest, HttpRequestPool};
use crate::rmap::ResourceMap;
use crate::service::{ServiceFactory, ServiceRequest, ServiceResponse};

type Guards = Vec<Box<dyn Guard>>;
type HttpService = BoxedService<ServiceRequest, ServiceResponse, Error>;
type HttpNewService = BoxedNewService<(), ServiceRequest, ServiceResponse, Error, ()>;
type BoxedResponse = Either<
    FutureResult<ServiceResponse, Error>,
    Box<dyn Future<Item = ServiceResponse, Error = Error>>,
>;
type FnDataFactory =
    Box<dyn Fn() -> Box<dyn Future<Item = Box<dyn DataFactory>, Error = ()>>>;

/// Service factory to convert `Request` to a `ServiceRequest<S>`.
/// It also executes data factories.
pub struct AppInit<T, B>
where
    T: NewService<
        Config = (),
        Request = ServiceRequest,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    pub(crate) endpoint: T,
    pub(crate) data: Rc<Vec<Box<dyn DataFactory>>>,
    pub(crate) data_factories: Rc<Vec<FnDataFactory>>,
    pub(crate) config: RefCell<AppConfig>,
    pub(crate) services: Rc<RefCell<Vec<Box<dyn ServiceFactory>>>>,
    pub(crate) default: Option<Rc<HttpNewService>>,
    pub(crate) factory_ref: Rc<RefCell<Option<AppRoutingFactory>>>,
    pub(crate) external: RefCell<Vec<ResourceDef>>,
}

impl<T, B> NewService for AppInit<T, B>
where
    T: NewService<
        Config = (),
        Request = ServiceRequest,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    type Config = ServerConfig;
    type Request = Request;
    type Response = ServiceResponse<B>;
    type Error = T::Error;
    type InitError = T::InitError;
    type Service = AppInitService<T::Service, B>;
    type Future = AppInitResult<T, B>;

    fn new_service(&self, cfg: &ServerConfig) -> Self::Future {
        // update resource default service
        let default = self.default.clone().unwrap_or_else(|| {
            Rc::new(boxed::new_service(service_fn(|req: ServiceRequest| {
                Ok(req.into_response(Response::NotFound().finish()))
            })))
        });

        // App config
        {
            let mut c = self.config.borrow_mut();
            let loc_cfg = Rc::get_mut(&mut c.0).unwrap();
            loc_cfg.secure = cfg.secure();
            loc_cfg.addr = cfg.local_addr();
        }

        let mut config = AppService::new(
            self.config.borrow().clone(),
            default.clone(),
            self.data.clone(),
        );

        // register services
        std::mem::replace(&mut *self.services.borrow_mut(), Vec::new())
            .into_iter()
            .for_each(|mut srv| srv.register(&mut config));

        let mut rmap = ResourceMap::new(ResourceDef::new(""));

        let (config, services) = config.into_services();

        // complete pipeline creation
        *self.factory_ref.borrow_mut() = Some(AppRoutingFactory {
            default,
            services: Rc::new(
                services
                    .into_iter()
                    .map(|(mut rdef, srv, guards, nested)| {
                        rmap.add(&mut rdef, nested);
                        (rdef, srv, RefCell::new(guards))
                    })
                    .collect(),
            ),
        });

        // external resources
        for mut rdef in std::mem::replace(&mut *self.external.borrow_mut(), Vec::new()) {
            rmap.add(&mut rdef, None);
        }

        // complete ResourceMap tree creation
        let rmap = Rc::new(rmap);
        rmap.finish(rmap.clone());

        AppInitResult {
            endpoint: None,
            endpoint_fut: self.endpoint.new_service(&()),
            data: self.data.clone(),
            data_factories: Vec::new(),
            data_factories_fut: self.data_factories.iter().map(|f| f()).collect(),
            config,
            rmap,
            _t: PhantomData,
        }
    }
}

pub struct AppInitResult<T, B>
where
    T: NewService,
{
    endpoint: Option<T::Service>,
    endpoint_fut: T::Future,
    rmap: Rc<ResourceMap>,
    config: AppConfig,
    data: Rc<Vec<Box<dyn DataFactory>>>,
    data_factories: Vec<Box<dyn DataFactory>>,
    data_factories_fut: Vec<Box<dyn Future<Item = Box<dyn DataFactory>, Error = ()>>>,
    _t: PhantomData<B>,
}

impl<T, B> Future for AppInitResult<T, B>
where
    T: NewService<
        Config = (),
        Request = ServiceRequest,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    type Item = AppInitService<T::Service, B>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // async data factories
        let mut idx = 0;
        while idx < self.data_factories_fut.len() {
            match self.data_factories_fut[idx].poll()? {
                Async::Ready(f) => {
                    self.data_factories.push(f);
                    let _ = self.data_factories_fut.remove(idx);
                }
                Async::NotReady => idx += 1,
            }
        }

        if self.endpoint.is_none() {
            if let Async::Ready(srv) = self.endpoint_fut.poll()? {
                self.endpoint = Some(srv);
            }
        }

        if self.endpoint.is_some() && self.data_factories_fut.is_empty() {
            // create app data container
            let mut data = Extensions::new();
            for f in self.data.iter() {
                f.create(&mut data);
            }

            for f in &self.data_factories {
                f.create(&mut data);
            }

            Ok(Async::Ready(AppInitService {
                service: self.endpoint.take().unwrap(),
                rmap: self.rmap.clone(),
                config: self.config.clone(),
                data: Rc::new(data),
                pool: HttpRequestPool::create(),
            }))
        } else {
            Ok(Async::NotReady)
        }
    }
}

/// Service to convert `Request` to a `ServiceRequest<S>`
pub struct AppInitService<T, B>
where
    T: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    service: T,
    rmap: Rc<ResourceMap>,
    config: AppConfig,
    data: Rc<Extensions>,
    pool: &'static HttpRequestPool,
}

impl<T, B> Service for AppInitService<T, B>
where
    T: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Request = Request;
    type Response = ServiceResponse<B>;
    type Error = T::Error;
    type Future = T::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let (head, payload) = req.into_parts();

        let req = if let Some(mut req) = self.pool.get_request() {
            let inner = Rc::get_mut(&mut req.0).unwrap();
            inner.path.get_mut().update(&head.uri);
            inner.path.reset();
            inner.head = head;
            inner.payload = payload;
            inner.app_data = self.data.clone();
            req
        } else {
            HttpRequest::new(
                Path::new(Url::new(head.uri.clone())),
                head,
                payload,
                self.rmap.clone(),
                self.config.clone(),
                self.data.clone(),
                self.pool,
            )
        };
        self.service.call(ServiceRequest::new(req))
    }
}

impl<T, B> Drop for AppInitService<T, B>
where
    T: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    fn drop(&mut self) {
        self.pool.clear();
    }
}

pub struct AppRoutingFactory {
    services: Rc<Vec<(ResourceDef, HttpNewService, RefCell<Option<Guards>>)>>,
    default: Rc<HttpNewService>,
}

impl NewService for AppRoutingFactory {
    type Config = ();
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = AppRouting;
    type Future = AppRoutingFactoryResponse;

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

type HttpServiceFut = Box<dyn Future<Item = HttpService, Error = ()>>;

/// Create app service
#[doc(hidden)]
pub struct AppRoutingFactoryResponse {
    fut: Vec<CreateAppRoutingItem>,
    default: Option<HttpService>,
    default_fut: Option<Box<dyn Future<Item = HttpService, Error = ()>>>,
}

enum CreateAppRoutingItem {
    Future(Option<ResourceDef>, Option<Guards>, HttpServiceFut),
    Service(ResourceDef, Option<Guards>, HttpService),
}

impl Future for AppRoutingFactoryResponse {
    type Item = AppRouting;
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

pub struct AppRouting {
    router: Router<HttpService, Guards>,
    ready: Option<(ServiceRequest, ResourceInfo)>,
    default: Option<HttpService>,
}

impl Service for AppRouting {
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = BoxedResponse;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        if self.ready.is_none() {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }

    fn call(&mut self, mut req: ServiceRequest) -> Self::Future {
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
            srv.call(req)
        } else if let Some(ref mut default) = self.default {
            default.call(req)
        } else {
            let req = req.into_parts().0;
            Either::A(ok(ServiceResponse::new(req, Response::NotFound().finish())))
        }
    }
}

/// Wrapper service for routing
pub struct AppEntry {
    factory: Rc<RefCell<Option<AppRoutingFactory>>>,
}

impl AppEntry {
    pub fn new(factory: Rc<RefCell<Option<AppRoutingFactory>>>) -> Self {
        AppEntry { factory }
    }
}

impl NewService for AppEntry {
    type Config = ();
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = AppRouting;
    type Future = AppRoutingFactoryResponse;

    fn new_service(&self, _: &()) -> Self::Future {
        self.factory.borrow_mut().as_mut().unwrap().new_service(&())
    }
}

#[cfg(test)]
mod tests {
    use actix_service::Service;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use crate::{test, web, App, HttpResponse};

    struct DropData(Arc<AtomicBool>);

    impl Drop for DropData {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn drop_data() {
        let data = Arc::new(AtomicBool::new(false));
        {
            let mut app = test::init_service(
                App::new()
                    .data(DropData(data.clone()))
                    .service(web::resource("/test").to(|| HttpResponse::Ok())),
            );
            let req = test::TestRequest::with_uri("/test").to_request();
            let _ = test::block_on(app.call(req)).unwrap();
        }
        assert!(data.load(Ordering::Relaxed));
    }
}
