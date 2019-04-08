use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::{Request, Response};
use actix_router::{Path, ResourceDef, ResourceInfo, Router, Url};
use actix_server_config::ServerConfig;
use actix_service::boxed::{self, BoxedNewService, BoxedService};
use actix_service::{fn_service, AndThen, NewService, Service, ServiceExt};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, Poll};

use crate::config::{AppConfig, ServiceConfig};
use crate::data::{DataFactory, DataFactoryResult};
use crate::error::Error;
use crate::guard::Guard;
use crate::request::{HttpRequest, HttpRequestPool};
use crate::rmap::ResourceMap;
use crate::service::{ServiceFactory, ServiceRequest, ServiceResponse};

type Guards = Vec<Box<Guard>>;
type HttpService<P> = BoxedService<ServiceRequest<P>, ServiceResponse, Error>;
type HttpNewService<P> =
    BoxedNewService<(), ServiceRequest<P>, ServiceResponse, Error, ()>;
type BoxedResponse = Either<
    FutureResult<ServiceResponse, Error>,
    Box<Future<Item = ServiceResponse, Error = Error>>,
>;

/// Service factory to convert `Request` to a `ServiceRequest<S>`.
/// It also executes data factories.
pub struct AppInit<C, T, P, B>
where
    C: NewService<Request = ServiceRequest, Response = ServiceRequest<P>>,
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    pub(crate) chain: C,
    pub(crate) endpoint: T,
    pub(crate) data: Vec<Box<DataFactory>>,
    pub(crate) config: RefCell<AppConfig>,
    pub(crate) services: RefCell<Vec<Box<ServiceFactory<P>>>>,
    pub(crate) default: Option<Rc<HttpNewService<P>>>,
    pub(crate) factory_ref: Rc<RefCell<Option<AppRoutingFactory<P>>>>,
    pub(crate) external: RefCell<Vec<ResourceDef>>,
}

impl<C, T, P: 'static, B> NewService<ServerConfig> for AppInit<C, T, P, B>
where
    C: NewService<
        Request = ServiceRequest,
        Response = ServiceRequest<P>,
        Error = Error,
        InitError = (),
    >,
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    type Request = Request;
    type Response = ServiceResponse<B>;
    type Error = C::Error;
    type InitError = C::InitError;
    type Service = AndThen<AppInitService<C::Service, P>, T::Service>;
    type Future = AppInitResult<C, T, P, B>;

    fn new_service(&self, cfg: &ServerConfig) -> Self::Future {
        // update resource default service
        let default = self.default.clone().unwrap_or_else(|| {
            Rc::new(boxed::new_service(fn_service(|req: ServiceRequest<P>| {
                Ok(req.into_response(Response::NotFound().finish()))
            })))
        });

        {
            let mut c = self.config.borrow_mut();
            let loc_cfg = Rc::get_mut(&mut c.0).unwrap();
            loc_cfg.secure = cfg.secure();
            loc_cfg.addr = cfg.local_addr();
        }

        let mut config =
            ServiceConfig::new(self.config.borrow().clone(), default.clone());

        // register services
        std::mem::replace(&mut *self.services.borrow_mut(), Vec::new())
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

        // external resources
        for mut rdef in std::mem::replace(&mut *self.external.borrow_mut(), Vec::new()) {
            rmap.add(&mut rdef, None);
        }

        // complete ResourceMap tree creation
        let rmap = Rc::new(rmap);
        rmap.finish(rmap.clone());

        AppInitResult {
            chain: None,
            chain_fut: self.chain.new_service(&()),
            endpoint: None,
            endpoint_fut: self.endpoint.new_service(&()),
            data: self.data.iter().map(|s| s.construct()).collect(),
            config: self.config.borrow().clone(),
            rmap,
            _t: PhantomData,
        }
    }
}

pub struct AppInitResult<C, T, P, B>
where
    C: NewService,
    T: NewService,
{
    chain: Option<C::Service>,
    endpoint: Option<T::Service>,
    chain_fut: C::Future,
    endpoint_fut: T::Future,
    rmap: Rc<ResourceMap>,
    data: Vec<Box<DataFactoryResult>>,
    config: AppConfig,
    _t: PhantomData<(P, B)>,
}

impl<C, T, P, B> Future for AppInitResult<C, T, P, B>
where
    C: NewService<
        Request = ServiceRequest,
        Response = ServiceRequest<P>,
        Error = Error,
        InitError = (),
    >,
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    type Item = AndThen<AppInitService<C::Service, P>, T::Service>;
    type Error = C::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut idx = 0;
        let mut extensions = self.config.0.extensions.borrow_mut();
        while idx < self.data.len() {
            if let Async::Ready(_) = self.data[idx].poll_result(&mut extensions)? {
                self.data.remove(idx);
            } else {
                idx += 1;
            }
        }

        if self.chain.is_none() {
            if let Async::Ready(srv) = self.chain_fut.poll()? {
                self.chain = Some(srv);
            }
        }

        if self.endpoint.is_none() {
            if let Async::Ready(srv) = self.endpoint_fut.poll()? {
                self.endpoint = Some(srv);
            }
        }

        if self.chain.is_some() && self.endpoint.is_some() {
            Ok(Async::Ready(
                AppInitService {
                    chain: self.chain.take().unwrap(),
                    rmap: self.rmap.clone(),
                    config: self.config.clone(),
                    pool: HttpRequestPool::create(),
                }
                .and_then(self.endpoint.take().unwrap()),
            ))
        } else {
            Ok(Async::NotReady)
        }
    }
}

/// Service to convert `Request` to a `ServiceRequest<S>`
pub struct AppInitService<C, P>
where
    C: Service<Request = ServiceRequest, Response = ServiceRequest<P>, Error = Error>,
{
    chain: C,
    rmap: Rc<ResourceMap>,
    config: AppConfig,
    pool: &'static HttpRequestPool,
}

impl<C, P> Service for AppInitService<C, P>
where
    C: Service<Request = ServiceRequest, Response = ServiceRequest<P>, Error = Error>,
{
    type Request = Request;
    type Response = ServiceRequest<P>;
    type Error = C::Error;
    type Future = C::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.chain.poll_ready()
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let (head, payload) = req.into_parts();

        let req = if let Some(mut req) = self.pool.get_request() {
            let inner = Rc::get_mut(&mut req.0).unwrap();
            inner.path.get_mut().update(&head.uri);
            inner.path.reset();
            inner.head = head;
            req
        } else {
            HttpRequest::new(
                Path::new(Url::new(head.uri.clone())),
                head,
                self.rmap.clone(),
                self.config.clone(),
                self.pool,
            )
        };
        self.chain.call(ServiceRequest::from_parts(req, payload))
    }
}

pub struct AppRoutingFactory<P> {
    services: Rc<Vec<(ResourceDef, HttpNewService<P>, RefCell<Option<Guards>>)>>,
    default: Rc<HttpNewService<P>>,
}

impl<P: 'static> NewService for AppRoutingFactory<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = Error;
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

impl<P> Service for AppRouting<P> {
    type Request = ServiceRequest<P>;
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
pub struct AppEntry<P> {
    factory: Rc<RefCell<Option<AppRoutingFactory<P>>>>,
}

impl<P> AppEntry<P> {
    pub fn new(factory: Rc<RefCell<Option<AppRoutingFactory<P>>>>) -> Self {
        AppEntry { factory }
    }
}

impl<P: 'static> NewService for AppEntry<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = AppRouting<P>;
    type Future = AppRoutingFactoryResponse<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        self.factory.borrow_mut().as_mut().unwrap().new_service(&())
    }
}

#[doc(hidden)]
pub struct AppChain;

impl NewService for AppChain {
    type Request = ServiceRequest;
    type Response = ServiceRequest;
    type Error = Error;
    type InitError = ();
    type Service = AppChain;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(AppChain)
    }
}

impl Service for AppChain {
    type Request = ServiceRequest;
    type Response = ServiceRequest;
    type Error = Error;
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
