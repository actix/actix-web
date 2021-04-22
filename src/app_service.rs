use std::cell::RefCell;
use std::rc::Rc;

use actix_http::{Extensions, Request};
use actix_router::{Path, ResourceDef, Router, Url};
use actix_service::boxed::{self, BoxService, BoxServiceFactory};
use actix_service::{fn_service, Service, ServiceFactory};
use futures_core::future::LocalBoxFuture;
use futures_util::future::join_all;

use crate::data::FnDataFactory;
use crate::error::Error;
use crate::guard::Guard;
use crate::request::{HttpRequest, HttpRequestPool};
use crate::rmap::ResourceMap;
use crate::service::{AppServiceFactory, ServiceRequest, ServiceResponse};
use crate::{
    config::{AppConfig, AppService},
    HttpResponse,
};

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
        // set AppService's default service to 404 NotFound
        // if no user defined default service exists.
        let default = self.default.clone().unwrap_or_else(|| {
            Rc::new(boxed::factory(fn_service(|req: ServiceRequest| async {
                Ok(req.into_response(HttpResponse::NotFound()))
            })))
        });

        // App config
        let mut config = AppService::new(config, default.clone());

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
            async_data_factories.iter().for_each(|factory| {
                factory.create(&mut app_data);
            });

            Ok(AppInitService {
                service,
                app_data: Rc::new(app_data),
                app_state: AppInitServiceState::new(rmap, config),
            })
        })
    }
}

/// Service that takes a [`Request`] and delegates to a service that take a [`ServiceRequest`].
pub struct AppInitService<T, B>
where
    T: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    service: T,
    app_data: Rc<Extensions>,
    app_state: Rc<AppInitServiceState>,
}

/// A collection of [`AppInitService`] state that shared across `HttpRequest`s.
pub(crate) struct AppInitServiceState {
    rmap: Rc<ResourceMap>,
    config: AppConfig,
    pool: HttpRequestPool,
}

impl AppInitServiceState {
    pub(crate) fn new(rmap: Rc<ResourceMap>, config: AppConfig) -> Rc<Self> {
        Rc::new(AppInitServiceState {
            rmap,
            config,
            // TODO: AppConfig can be used to pass user defined HttpRequestPool
            // capacity.
            pool: HttpRequestPool::default(),
        })
    }

    #[inline]
    pub(crate) fn rmap(&self) -> &ResourceMap {
        &*self.rmap
    }

    #[inline]
    pub(crate) fn config(&self) -> &AppConfig {
        &self.config
    }

    #[inline]
    pub(crate) fn pool(&self) -> &HttpRequestPool {
        &self.pool
    }
}

impl<T, B> Service<Request> for AppInitService<T, B>
where
    T: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Response = ServiceResponse<B>;
    type Error = T::Error;
    type Future = T::Future;

    actix_service::forward_ready!(service);

    fn call(&self, req: Request) -> Self::Future {
        let (head, payload) = req.into_parts();

        let req = if let Some(mut req) = self.app_state.pool().pop() {
            let inner = Rc::get_mut(&mut req.inner).unwrap();
            inner.path.get_mut().update(&head.uri);
            inner.path.reset();
            inner.head = head;
            req
        } else {
            HttpRequest::new(
                Path::new(Url::new(head.uri.clone())),
                head,
                self.app_state.clone(),
                self.app_data.clone(),
            )
        };
        self.service.call(ServiceRequest::new(req, payload))
    }
}

impl<T, B> Drop for AppInitService<T, B>
where
    T: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    fn drop(&mut self) {
        self.app_state.pool().clear();
    }
}

pub struct AppRoutingFactory {
    services: Rc<[(ResourceDef, HttpNewService, RefCell<Option<Guards>>)]>,
    default: Rc<HttpNewService>,
}

impl ServiceFactory<ServiceRequest> for AppRoutingFactory {
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = AppRouting;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        // construct all services factory future with it's resource def and guards.
        let factory_fut = join_all(self.services.iter().map(|(path, factory, guards)| {
            let path = path.clone();
            let guards = guards.borrow_mut().take();
            let factory_fut = factory.new_service(());
            async move {
                let service = factory_fut.await?;
                Ok((path, guards, service))
            }
        }));

        // construct default service factory future
        let default_fut = self.default.new_service(());

        Box::pin(async move {
            let default = default_fut.await?;

            // build router from the factory future result.
            let router = factory_fut
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?
                .drain(..)
                .fold(Router::build(), |mut router, (path, guards, service)| {
                    router.rdef(path, service).2 = guards;
                    router
                })
                .finish();

            Ok(AppRouting { router, default })
        })
    }
}

pub struct AppRouting {
    router: Router<HttpService, Guards>,
    default: HttpService,
}

impl Service<ServiceRequest> for AppRouting {
    type Response = ServiceResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let res = self.router.recognize_checked(&mut req, |req, guards| {
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
        } else {
            self.default.call(req)
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
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        self.factory.borrow_mut().as_mut().unwrap().new_service(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use actix_service::Service;

    use crate::test::{init_service, TestRequest};
    use crate::{web, App, HttpResponse};

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
            let app = init_service(
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
