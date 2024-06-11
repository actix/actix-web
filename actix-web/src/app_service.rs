use std::{cell::RefCell, mem, rc::Rc};

use actix_http::Request;
use actix_router::{Path, ResourceDef, Router, Url};
use actix_service::{boxed, fn_service, Service, ServiceFactory};
use futures_core::future::LocalBoxFuture;
use futures_util::future::join_all;

use crate::{
    body::BoxBody,
    config::{AppConfig, AppService},
    data::FnDataFactory,
    dev::Extensions,
    guard::Guard,
    request::{HttpRequest, HttpRequestPool},
    rmap::ResourceMap,
    service::{
        AppServiceFactory, BoxedHttpService, BoxedHttpServiceFactory, ServiceRequest,
        ServiceResponse,
    },
    Error, HttpResponse,
};

/// Service factory to convert [`Request`] to a [`ServiceRequest<S>`].
///
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
    pub(crate) default: Option<Rc<BoxedHttpServiceFactory>>,
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

        // create App config to pass to child services
        let mut config = AppService::new(config, default.clone());

        // register services
        mem::take(&mut *self.services.borrow_mut())
            .into_iter()
            .for_each(|mut srv| srv.register(&mut config));

        let mut rmap = ResourceMap::new(ResourceDef::prefix(""));

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
        for mut rdef in mem::take(&mut *self.external.borrow_mut()) {
            rmap.add(&mut rdef, None);
        }

        // complete ResourceMap tree creation
        let rmap = Rc::new(rmap);
        ResourceMap::finish(&rmap);

        // construct all async data factory futures
        let factory_futs = join_all(self.async_data_factories.iter().map(|f| f()));

        // construct app service and middleware service factory future.
        let endpoint_fut = self.endpoint.new_service(());

        // take extensions or create new one as app data container.
        let mut app_data = self.extensions.borrow_mut().take().unwrap_or_default();

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
            for factory in &async_data_factories {
                factory.create(&mut app_data);
            }

            Ok(AppInitService {
                service,
                app_data: Rc::new(app_data),
                app_state: AppInitServiceState::new(rmap, config),
            })
        })
    }
}

/// The [`Service`] that is passed to `actix-http`'s server builder.
///
/// Wraps a service receiving a [`ServiceRequest`] into one receiving a [`Request`].
pub struct AppInitService<T, B>
where
    T: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    service: T,
    app_data: Rc<Extensions>,
    app_state: Rc<AppInitServiceState>,
}

/// A collection of state for [`AppInitService`] that is shared across [`HttpRequest`]s.
pub(crate) struct AppInitServiceState {
    rmap: Rc<ResourceMap>,
    config: AppConfig,
    pool: HttpRequestPool,
}

impl AppInitServiceState {
    /// Constructs state collection from resource map and app config.
    pub(crate) fn new(rmap: Rc<ResourceMap>, config: AppConfig) -> Rc<Self> {
        Rc::new(AppInitServiceState {
            rmap,
            config,
            pool: HttpRequestPool::default(),
        })
    }

    /// Returns a reference to the application's resource map.
    #[inline]
    pub(crate) fn rmap(&self) -> &ResourceMap {
        &self.rmap
    }

    /// Returns a reference to the application's configuration.
    #[inline]
    pub(crate) fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Returns a reference to the application's request pool.
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

    fn call(&self, mut req: Request) -> Self::Future {
        let extensions = Rc::new(RefCell::new(req.take_req_data()));
        let conn_data = req.take_conn_data();
        let (head, payload) = req.into_parts();

        let req = match self.app_state.pool().pop() {
            Some(mut req) => {
                let inner = Rc::get_mut(&mut req.inner).unwrap();
                inner.path.get_mut().update(&head.uri);
                inner.path.reset();
                inner.head = head;
                inner.conn_data = conn_data;
                inner.extensions = extensions;
                req
            }

            None => HttpRequest::new(
                Path::new(Url::new(head.uri.clone())),
                head,
                Rc::clone(&self.app_state),
                Rc::clone(&self.app_data),
                conn_data,
                extensions,
            ),
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
    #[allow(clippy::type_complexity)]
    services: Rc<
        [(
            ResourceDef,
            BoxedHttpServiceFactory,
            RefCell<Option<Vec<Box<dyn Guard>>>>,
        )],
    >,
    default: Rc<BoxedHttpServiceFactory>,
}

impl ServiceFactory<ServiceRequest> for AppRoutingFactory {
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = AppRouting;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        // construct all services factory future with its resource def and guards.
        let factory_fut = join_all(self.services.iter().map(|(path, factory, guards)| {
            let path = path.clone();
            let guards = guards.borrow_mut().take().unwrap_or_default();
            let factory_fut = factory.new_service(());
            async move {
                factory_fut
                    .await
                    .map(move |service| (path, guards, service))
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
                    router.push(path, service, guards);
                    router
                })
                .finish();

            Ok(AppRouting { router, default })
        })
    }
}

/// The Actix Web router default entry point.
pub struct AppRouting {
    router: Router<BoxedHttpService, Vec<Box<dyn Guard>>>,
    default: BoxedHttpService,
}

impl Service<ServiceRequest> for AppRouting {
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let res = self.router.recognize_fn(&mut req, |req, guards| {
            let guard_ctx = req.guard_ctx();
            guards.iter().all(|guard| guard.check(&guard_ctx))
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
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use actix_service::Service;

    use crate::{
        test::{init_service, TestRequest},
        web, App, HttpResponse,
    };

    struct DropData(Arc<AtomicBool>);

    impl Drop for DropData {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    // allow deprecated App::data
    #[allow(deprecated)]
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
