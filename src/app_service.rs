use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use actix_http::{Extensions, Request, Response};
use actix_router::{Path, ResourceDef, Router, Url};
use actix_service::boxed::{self, BoxService, BoxServiceFactory};
use actix_service::{fn_service, Service, ServiceFactory};
use futures_util::future::join_all;
use futures_core::future::LocalBoxFuture;

use crate::config::{AppConfig, AppService};
use crate::data::{DataFactory, FnDataFactory};
use crate::error::Error;
use crate::guard::Guard;
use crate::request::{HttpRequest, HttpRequestPool};
use crate::rmap::ResourceMap;
use crate::service::{AppServiceFactory, ServiceRequest, ServiceResponse};

type Guards = Vec<Box<dyn Guard>>;
type HttpService = BoxService<ServiceRequest, ServiceResponse, Error>;
type HttpNewService = BoxServiceFactory<(), ServiceRequest, ServiceResponse, Error, ()>;

/// Service factory to convert `Request` to a `ServiceRequest<S>`.
/// It also executes data factories.
pub struct AppInit<T, B>
where
    T: ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    pub(crate) endpoint: T,
    pub(crate) extensions: RefCell<Option<Extensions>>,
    pub(crate) data_factories: Rc<[Box<dyn DataFactory>]>,
    pub(crate) async_data_factories: Rc<[FnDataFactory]>,
    pub(crate) services: Rc<RefCell<Vec<Box<dyn AppServiceFactory>>>>,
    pub(crate) default: Option<Rc<HttpNewService>>,
    pub(crate) factory_ref: Rc<RefCell<Option<AppRoutingFactory>>>,
    pub(crate) external: RefCell<Vec<ResourceDef>>,
}

impl<T, B> ServiceFactory<Request> for AppInit<T, B>
where
    T: ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
    T::Future: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = T::Error;
    type Config = AppConfig;
    type Service = AppInitService<T::Service, B>;
    type InitError = T::InitError;
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, config: AppConfig) -> Self::Future {
        // update resource default service
        let default = self.default.clone().unwrap_or_else(|| {
            Rc::new(boxed::factory(fn_service(|req: ServiceRequest| async {
                Ok(req.into_response(Response::NotFound().finish()))
            })))
        });

        // App config
        let mut config =
            AppService::new(config, default.clone(), self.data_factories.clone());

        // register services
        std::mem::take(&mut *self.services.borrow_mut())
            .into_iter()
            .for_each(|mut srv| srv.register(&mut config));

        let mut rmap = ResourceMap::new(ResourceDef::new(""));

        let (config, services) = config.into_services();

        // complete pipeline creation.
        *self.factory_ref.borrow_mut() = Some(AppRoutingFactory {
            default,
            services: services
                .into_iter()
                .map(|(mut rdef, srv, guards, nested)| {
                    rmap.add(&mut rdef, nested);
                    (rdef, srv, RefCell::new(guards))
                })
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .into(),
        });

        // external resources
        for mut rdef in std::mem::take(&mut *self.external.borrow_mut()) {
            rmap.add(&mut rdef, None);
        }

        // complete ResourceMap tree creation
        let rmap = Rc::new(rmap);
        rmap.finish(rmap.clone());

        // construct all async data factory futures
        let factory_futs = join_all(self.async_data_factories.iter().map(|f| f()));

        // construct app service and middleware service factory future.
        let endpoint_fut = self.endpoint.new_service(());

        // take extensions or create new one as app data container.
        let mut app_data = self
            .extensions
            .borrow_mut()
            .take()
            .unwrap_or_else(Extensions::new);

        let data_factories = self.data_factories.clone();

        Box::pin(async move {
            // async data factories
            let async_data_factories = factory_futs
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ())?;

            // app service and middleware
            let service = endpoint_fut.await?;

            // populate app data container from (async) data factories.
            data_factories
                .iter()
                .chain(&async_data_factories)
                .for_each(|factory| {
                    factory.create(&mut app_data);
                });

            Ok(AppInitService {
                service,
                rmap,
                config,
                app_data: Rc::new(app_data),
                pool: HttpRequestPool::create(),
            })
        })
    }
}

/// Service to convert `Request` to a `ServiceRequest<S>`
pub struct AppInitService<T, B>
where
    T: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    service: T,
    rmap: Rc<ResourceMap>,
    config: AppConfig,
    app_data: Rc<Extensions>,
    pool: &'static HttpRequestPool,
}

impl<T, B> Service<Request> for AppInitService<T, B>
where
    T: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Response = ServiceResponse<B>;
    type Error = T::Error;
    type Future = T::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let (head, payload) = req.into_parts();

        let req = if let Some(mut req) = self.pool.get_request() {
            let inner = Rc::get_mut(&mut req.inner).unwrap();
            inner.path.get_mut().update(&head.uri);
            inner.path.reset();
            inner.head = head;
            inner.payload = payload;
            req
        } else {
            HttpRequest::new(
                Path::new(Url::new(head.uri.clone())),
                head,
                payload,
                self.rmap.clone(),
                self.config.clone(),
                self.app_data.clone(),
                self.pool,
            )
        };
        self.service.call(ServiceRequest::new(req))
    }
}

impl<T, B> Drop for AppInitService<T, B>
where
    T: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    fn drop(&mut self) {
        self.pool.clear();
    }
}

pub struct AppRoutingFactory {
    services: Rc<[(ResourceDef, HttpNewService, RefCell<Option<Guards>>)]>,
    default: Rc<HttpNewService>,
}

impl ServiceFactory<ServiceRequest> for AppRoutingFactory {
    type Config = ();
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = AppRouting;
    type Future = AppRoutingFactoryResponse;

    fn new_service(&self, _: ()) -> Self::Future {
        AppRoutingFactoryResponse {
            fut: self
                .services
                .iter()
                .map(|(path, service, guards)| {
                    CreateAppRoutingItem::Future(
                        Some(path.clone()),
                        guards.borrow_mut().take(),
                        Box::pin(service.new_service(())),
                    )
                })
                .collect(),
            default: None,
            default_fut: Some(self.default.new_service(())),
        }
    }
}

type HttpServiceFut = LocalBoxFuture<'static, Result<HttpService, ()>>;

/// Create app service
#[doc(hidden)]
pub struct AppRoutingFactoryResponse {
    fut: Vec<CreateAppRoutingItem>,
    default: Option<HttpService>,
    default_fut: Option<HttpServiceFut>,
}

enum CreateAppRoutingItem {
    Future(Option<ResourceDef>, Option<Guards>, HttpServiceFut),
    Service(ResourceDef, Option<Guards>, HttpService),
}

impl Future for AppRoutingFactoryResponse {
    type Output = Result<AppRouting, ()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut done = true;

        if let Some(ref mut fut) = self.default_fut {
            match Pin::new(fut).poll(cx)? {
                Poll::Ready(default) => self.default = Some(default),
                Poll::Pending => done = false,
            }
        }

        // poll http services
        for item in &mut self.fut {
            let res = match item {
                CreateAppRoutingItem::Future(
                    ref mut path,
                    ref mut guards,
                    ref mut fut,
                ) => match Pin::new(fut).poll(cx) {
                    Poll::Ready(Ok(service)) => {
                        Some((path.take().unwrap(), guards.take(), service))
                    }
                    Poll::Ready(Err(_)) => return Poll::Ready(Err(())),
                    Poll::Pending => {
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
            Poll::Ready(Ok(AppRouting {
                router: router.finish(),
                default: self.default.take(),
            }))
        } else {
            Poll::Pending
        }
    }
}

pub struct AppRouting {
    router: Router<HttpService, Guards>,
    default: Option<HttpService>,
}

impl Service<ServiceRequest> for AppRouting {
    type Response = ServiceResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

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
            Box::pin(async {
                Ok(ServiceResponse::new(req, Response::NotFound().finish()))
            })
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

impl ServiceFactory<ServiceRequest> for AppEntry {
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = AppRouting;
    type InitError = ();
    type Future = AppRoutingFactoryResponse;

    fn new_service(&self, _: ()) -> Self::Future {
        self.factory.borrow_mut().as_mut().unwrap().new_service(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use crate::test::{init_service, TestRequest};
    use crate::{web, App, HttpResponse};
    use actix_service::Service;

    struct DropData(Arc<AtomicBool>);

    impl Drop for DropData {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    #[actix_rt::test]
    async fn test_drop_data() {
        let data = Arc::new(AtomicBool::new(false));

        {
            let mut app = init_service(
                App::new()
                    .data(DropData(data.clone()))
                    .service(web::resource("/test").to(HttpResponse::Ok)),
            )
            .await;
            let req = TestRequest::with_uri("/test").to_request();
            let _ = app.call(req).await.unwrap();
        }
        assert!(data.load(Ordering::Relaxed));
    }
}
