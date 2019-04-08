use std::cell::RefCell;
use std::rc::Rc;

use actix_http::Response;
use actix_router::{ResourceDef, ResourceInfo, Router};
use actix_service::boxed::{self, BoxedNewService, BoxedService};
use actix_service::{
    ApplyTransform, IntoNewService, IntoTransform, NewService, Service, Transform,
};
use futures::future::{ok, Either, Future, FutureResult};
use futures::{Async, IntoFuture, Poll};

use crate::dev::{HttpServiceFactory, ServiceConfig};
use crate::error::Error;
use crate::guard::Guard;
use crate::resource::Resource;
use crate::rmap::ResourceMap;
use crate::route::Route;
use crate::service::{
    ServiceFactory, ServiceFactoryWrapper, ServiceRequest, ServiceResponse,
};

type Guards = Vec<Box<Guard>>;
type HttpService<P> = BoxedService<ServiceRequest<P>, ServiceResponse, Error>;
type HttpNewService<P> =
    BoxedNewService<(), ServiceRequest<P>, ServiceResponse, Error, ()>;
type BoxedResponse = Either<
    FutureResult<ServiceResponse, Error>,
    Box<Future<Item = ServiceResponse, Error = Error>>,
>;

/// Resources scope.
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
/// use actix_web::{web, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::scope("/{project_id}/")
///             .service(web::resource("/path1").to(|| HttpResponse::Ok()))
///             .service(web::resource("/path2").route(web::get().to(|| HttpResponse::Ok())))
///             .service(web::resource("/path3").route(web::head().to(|| HttpResponse::MethodNotAllowed())))
///     );
/// }
/// ```
///
/// In the above example three routes get registered:
///  * /{project_id}/path1 - reponds to all http method
///  * /{project_id}/path2 - `GET` requests
///  * /{project_id}/path3 - `HEAD` requests
///
pub struct Scope<P, T = ScopeEndpoint<P>> {
    endpoint: T,
    rdef: String,
    services: Vec<Box<ServiceFactory<P>>>,
    guards: Vec<Box<Guard>>,
    default: Rc<RefCell<Option<Rc<HttpNewService<P>>>>>,
    factory_ref: Rc<RefCell<Option<ScopeFactory<P>>>>,
}

impl<P: 'static> Scope<P> {
    /// Create a new scope
    pub fn new(path: &str) -> Scope<P> {
        let fref = Rc::new(RefCell::new(None));
        Scope {
            endpoint: ScopeEndpoint::new(fref.clone()),
            rdef: path.to_string(),
            guards: Vec::new(),
            services: Vec::new(),
            default: Rc::new(RefCell::new(None)),
            factory_ref: fref,
        }
    }
}

impl<P, T> Scope<P, T>
where
    P: 'static,
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse,
        Error = Error,
        InitError = (),
    >,
{
    /// Add match guard to a scope.
    ///
    /// ```rust
    /// use actix_web::{web, guard, App, HttpRequest, HttpResponse};
    ///
    /// fn index(data: web::Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::scope("/app")
    ///             .guard(guard::Header("content-type", "text/plain"))
    ///             .route("/test1", web::get().to(index))
    ///             .route("/test2", web::post().to(|r: HttpRequest| {
    ///                 HttpResponse::MethodNotAllowed()
    ///             }))
    ///     );
    /// }
    /// ```
    pub fn guard<G: Guard + 'static>(mut self, guard: G) -> Self {
        self.guards.push(Box::new(guard));
        self
    }

    /// Register http service.
    ///
    /// This is similar to `App's` service registration.
    ///
    /// Actix web provides several services implementations:
    ///
    /// * *Resource* is an entry in resource table which corresponds to requested URL.
    /// * *Scope* is a set of resources with common root path.
    /// * "StaticFiles" is a service for static files support
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpRequest};
    ///
    /// struct AppState;
    ///
    /// fn index(req: HttpRequest) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::scope("/app").service(
    ///             web::scope("/v1")
    ///                 .service(web::resource("/test1").to(index)))
    ///     );
    /// }
    /// ```
    pub fn service<F>(mut self, factory: F) -> Self
    where
        F: HttpServiceFactory<P> + 'static,
    {
        self.services
            .push(Box::new(ServiceFactoryWrapper::new(factory)));
        self
    }

    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `Scope::service()` method.
    /// This method can be called multiple times, in that case
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
    ///     let app = App::new().service(
    ///         web::scope("/app")
    ///             .route("/test1", web::get().to(index))
    ///             .route("/test2", web::post().to(|| HttpResponse::MethodNotAllowed()))
    ///     );
    /// }
    /// ```
    pub fn route(self, path: &str, mut route: Route<P>) -> Self {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Default resource to be used if no matching route could be found.
    ///
    /// If default resource is not registered, app's default resource is being used.
    pub fn default_resource<F, U>(mut self, f: F) -> Self
    where
        F: FnOnce(Resource<P>) -> Resource<P, U>,
        U: NewService<
                Request = ServiceRequest<P>,
                Response = ServiceResponse,
                Error = Error,
                InitError = (),
            > + 'static,
    {
        // create and configure default resource
        self.default = Rc::new(RefCell::new(Some(Rc::new(boxed::new_service(
            f(Resource::new("")).into_new_service().map_init_err(|_| ()),
        )))));

        self
    }

    /// Registers middleware, in the form of a middleware component (type),
    /// that runs during inbound processing in the request
    /// lifecycle (request -> response), modifying request as
    /// necessary, across all requests managed by the *Scope*.  Scope-level
    /// middleware is more limited in what it can modify, relative to Route or
    /// Application level middleware, in that Scope-level middleware can not modify
    /// ServiceResponse.
    ///
    /// Use middleware when you need to read or modify *every* request in some way.
    pub fn wrap<M, F>(
        self,
        mw: F,
    ) -> Scope<
        P,
        impl NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        >,
    >
    where
        M: Transform<
            T::Service,
            Request = ServiceRequest<P>,
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        >,
        F: IntoTransform<M, T::Service>,
    {
        let endpoint = ApplyTransform::new(mw, self.endpoint);
        Scope {
            endpoint,
            rdef: self.rdef,
            guards: self.guards,
            services: self.services,
            default: self.default,
            factory_ref: self.factory_ref,
        }
    }

    /// Registers middleware, in the form of a closure, that runs during inbound
    /// processing in the request lifecycle (request -> response), modifying
    /// request as necessary, across all requests managed by the *Scope*.  
    /// Scope-level middleware is more limited in what it can modify, relative
    /// to Route or Application level middleware, in that Scope-level middleware
    /// can not modify ServiceResponse.
    ///
    /// ```rust
    /// use actix_service::Service;
    /// # use futures::Future;
    /// use actix_web::{web, App};
    /// use actix_web::http::{header::CONTENT_TYPE, HeaderValue};
    ///
    /// fn index() -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::scope("/app")
    ///             .wrap_fn(|req, srv|
    ///                 srv.call(req).map(|mut res| {
    ///                     res.headers_mut().insert(
    ///                        CONTENT_TYPE, HeaderValue::from_static("text/plain"),
    ///                     );
    ///                     res
    ///                 }))
    ///             .route("/index.html", web::get().to(index)));
    /// }
    /// ```
    pub fn wrap_fn<F, R>(
        self,
        mw: F,
    ) -> Scope<
        P,
        impl NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        >,
    >
    where
        F: FnMut(ServiceRequest<P>, &mut T::Service) -> R + Clone,
        R: IntoFuture<Item = ServiceResponse, Error = Error>,
    {
        self.wrap(mw)
    }
}

impl<P, T> HttpServiceFactory<P> for Scope<P, T>
where
    P: 'static,
    T: NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        > + 'static,
{
    fn register(self, config: &mut ServiceConfig<P>) {
        // update default resource if needed
        if self.default.borrow().is_none() {
            *self.default.borrow_mut() = Some(config.default_service());
        }

        // register nested services
        let mut cfg = config.clone_config();
        self.services
            .into_iter()
            .for_each(|mut srv| srv.register(&mut cfg));

        let mut rmap = ResourceMap::new(ResourceDef::root_prefix(&self.rdef));

        // complete scope pipeline creation
        *self.factory_ref.borrow_mut() = Some(ScopeFactory {
            default: self.default.clone(),
            services: Rc::new(
                cfg.into_services()
                    .into_iter()
                    .map(|(mut rdef, srv, guards, nested)| {
                        rmap.add(&mut rdef, nested);
                        (rdef, srv, RefCell::new(guards))
                    })
                    .collect(),
            ),
        });

        // get guards
        let guards = if self.guards.is_empty() {
            None
        } else {
            Some(self.guards)
        };

        // register final service
        config.register_service(
            ResourceDef::root_prefix(&self.rdef),
            guards,
            self.endpoint,
            Some(Rc::new(rmap)),
        )
    }
}

pub struct ScopeFactory<P> {
    services: Rc<Vec<(ResourceDef, HttpNewService<P>, RefCell<Option<Guards>>)>>,
    default: Rc<RefCell<Option<Rc<HttpNewService<P>>>>>,
}

impl<P: 'static> NewService for ScopeFactory<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = ScopeService<P>;
    type Future = ScopeFactoryResponse<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        let default_fut = if let Some(ref default) = *self.default.borrow() {
            Some(default.new_service(&()))
        } else {
            None
        };

        ScopeFactoryResponse {
            fut: self
                .services
                .iter()
                .map(|(path, service, guards)| {
                    CreateScopeServiceItem::Future(
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

/// Create scope service
#[doc(hidden)]
pub struct ScopeFactoryResponse<P> {
    fut: Vec<CreateScopeServiceItem<P>>,
    default: Option<HttpService<P>>,
    default_fut: Option<Box<Future<Item = HttpService<P>, Error = ()>>>,
}

type HttpServiceFut<P> = Box<Future<Item = HttpService<P>, Error = ()>>;

enum CreateScopeServiceItem<P> {
    Future(Option<ResourceDef>, Option<Guards>, HttpServiceFut<P>),
    Service(ResourceDef, Option<Guards>, HttpService<P>),
}

impl<P> Future for ScopeFactoryResponse<P> {
    type Item = ScopeService<P>;
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
                CreateScopeServiceItem::Future(
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
                CreateScopeServiceItem::Service(_, _, _) => continue,
            };

            if let Some((path, guards, service)) = res {
                *item = CreateScopeServiceItem::Service(path, guards, service);
            }
        }

        if done {
            let router = self
                .fut
                .drain(..)
                .fold(Router::build(), |mut router, item| {
                    match item {
                        CreateScopeServiceItem::Service(path, guards, service) => {
                            router.rdef(path, service).2 = guards;
                        }
                        CreateScopeServiceItem::Future(_, _, _) => unreachable!(),
                    }
                    router
                });
            Ok(Async::Ready(ScopeService {
                router: router.finish(),
                default: self.default.take(),
                _ready: None,
            }))
        } else {
            Ok(Async::NotReady)
        }
    }
}

pub struct ScopeService<P> {
    router: Router<HttpService<P>, Vec<Box<Guard>>>,
    default: Option<HttpService<P>>,
    _ready: Option<(ServiceRequest<P>, ResourceInfo)>,
}

impl<P> Service for ScopeService<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = Either<BoxedResponse, FutureResult<Self::Response, Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
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
            let req = req.into_parts().0;
            Either::B(ok(ServiceResponse::new(req, Response::NotFound().finish())))
        }
    }
}

#[doc(hidden)]
pub struct ScopeEndpoint<P> {
    factory: Rc<RefCell<Option<ScopeFactory<P>>>>,
}

impl<P> ScopeEndpoint<P> {
    fn new(factory: Rc<RefCell<Option<ScopeFactory<P>>>>) -> Self {
        ScopeEndpoint { factory }
    }
}

impl<P: 'static> NewService for ScopeEndpoint<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = ScopeService<P>;
    type Future = ScopeFactoryResponse<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        self.factory.borrow_mut().as_mut().unwrap().new_service(&())
    }
}

#[cfg(test)]
mod tests {
    use actix_service::Service;
    use bytes::Bytes;
    use futures::{Future, IntoFuture};

    use crate::dev::{Body, ResponseBody};
    use crate::http::{header, HeaderValue, Method, StatusCode};
    use crate::service::{ServiceRequest, ServiceResponse};
    use crate::test::{block_on, call_success, init_service, TestRequest};
    use crate::{guard, web, App, Error, HttpRequest, HttpResponse};

    #[test]
    fn test_scope() {
        let mut srv = init_service(
            App::new().service(
                web::scope("/app")
                    .service(web::resource("/path1").to(|| HttpResponse::Ok())),
            ),
        );

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_scope_root() {
        let mut srv = init_service(
            App::new().service(
                web::scope("/app")
                    .service(web::resource("").to(|| HttpResponse::Ok()))
                    .service(web::resource("/").to(|| HttpResponse::Created())),
            ),
        );

        let req = TestRequest::with_uri("/app").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_scope_root2() {
        let mut srv = init_service(App::new().service(
            web::scope("/app/").service(web::resource("").to(|| HttpResponse::Ok())),
        ));

        let req = TestRequest::with_uri("/app").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_scope_root3() {
        let mut srv = init_service(App::new().service(
            web::scope("/app/").service(web::resource("/").to(|| HttpResponse::Ok())),
        ));

        let req = TestRequest::with_uri("/app").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_scope_route() {
        let mut srv = init_service(
            App::new().service(
                web::scope("app")
                    .route("/path1", web::get().to(|| HttpResponse::Ok()))
                    .route("/path1", web::delete().to(|| HttpResponse::Ok())),
            ),
        );

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::DELETE)
            .to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_scope_route_without_leading_slash() {
        let mut srv = init_service(
            App::new().service(
                web::scope("app").service(
                    web::resource("path1")
                        .route(web::get().to(|| HttpResponse::Ok()))
                        .route(web::delete().to(|| HttpResponse::Ok())),
                ),
            ),
        );

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::DELETE)
            .to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn test_scope_guard() {
        let mut srv = init_service(
            App::new().service(
                web::scope("/app")
                    .guard(guard::Get())
                    .service(web::resource("/path1").to(|| HttpResponse::Ok())),
            ),
        );

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::GET)
            .to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_scope_variable_segment() {
        let mut srv =
            init_service(App::new().service(web::scope("/ab-{project}").service(
                web::resource("/path1").to(|r: HttpRequest| {
                    HttpResponse::Ok()
                        .body(format!("project: {}", &r.match_info()["project"]))
                }),
            )));

        let req = TestRequest::with_uri("/ab-project1/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        match resp.response().body() {
            ResponseBody::Body(Body::Bytes(ref b)) => {
                let bytes: Bytes = b.clone().into();
                assert_eq!(bytes, Bytes::from_static(b"project: project1"));
            }
            _ => panic!(),
        }

        let req = TestRequest::with_uri("/aa-project1/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_nested_scope() {
        let mut srv = init_service(
            App::new().service(
                web::scope("/app")
                    .service(web::scope("/t1").service(
                        web::resource("/path1").to(|| HttpResponse::Created()),
                    )),
            ),
        );

        let req = TestRequest::with_uri("/app/t1/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_nested_scope_no_slash() {
        let mut srv = init_service(
            App::new().service(
                web::scope("/app")
                    .service(web::scope("t1").service(
                        web::resource("/path1").to(|| HttpResponse::Created()),
                    )),
            ),
        );

        let req = TestRequest::with_uri("/app/t1/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_nested_scope_root() {
        let mut srv = init_service(
            App::new().service(
                web::scope("/app").service(
                    web::scope("/t1")
                        .service(web::resource("").to(|| HttpResponse::Ok()))
                        .service(web::resource("/").to(|| HttpResponse::Created())),
                ),
            ),
        );

        let req = TestRequest::with_uri("/app/t1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/t1/").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_nested_scope_filter() {
        let mut srv = init_service(
            App::new().service(
                web::scope("/app").service(
                    web::scope("/t1")
                        .guard(guard::Get())
                        .service(web::resource("/path1").to(|| HttpResponse::Ok())),
                ),
            ),
        );

        let req = TestRequest::with_uri("/app/t1/path1")
            .method(Method::POST)
            .to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/t1/path1")
            .method(Method::GET)
            .to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_nested_scope_with_variable_segment() {
        let mut srv = init_service(App::new().service(web::scope("/app").service(
            web::scope("/{project_id}").service(web::resource("/path1").to(
                |r: HttpRequest| {
                    HttpResponse::Created()
                        .body(format!("project: {}", &r.match_info()["project_id"]))
                },
            )),
        )));

        let req = TestRequest::with_uri("/app/project_1/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        match resp.response().body() {
            ResponseBody::Body(Body::Bytes(ref b)) => {
                let bytes: Bytes = b.clone().into();
                assert_eq!(bytes, Bytes::from_static(b"project: project_1"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_nested2_scope_with_variable_segment() {
        let mut srv = init_service(App::new().service(web::scope("/app").service(
            web::scope("/{project}").service(web::scope("/{id}").service(
                web::resource("/path1").to(|r: HttpRequest| {
                    HttpResponse::Created().body(format!(
                        "project: {} - {}",
                        &r.match_info()["project"],
                        &r.match_info()["id"],
                    ))
                }),
            )),
        )));

        let req = TestRequest::with_uri("/app/test/1/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        match resp.response().body() {
            ResponseBody::Body(Body::Bytes(ref b)) => {
                let bytes: Bytes = b.clone().into();
                assert_eq!(bytes, Bytes::from_static(b"project: test - 1"));
            }
            _ => panic!(),
        }

        let req = TestRequest::with_uri("/app/test/1/path2").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_default_resource() {
        let mut srv = init_service(
            App::new().service(
                web::scope("/app")
                    .service(web::resource("/path1").to(|| HttpResponse::Ok()))
                    .default_resource(|r| r.to(|| HttpResponse::BadRequest())),
            ),
        );

        let req = TestRequest::with_uri("/app/path2").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/path2").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_default_resource_propagation() {
        let mut srv = init_service(
            App::new()
                .service(
                    web::scope("/app1")
                        .default_resource(|r| r.to(|| HttpResponse::BadRequest())),
                )
                .service(web::scope("/app2"))
                .default_resource(|r| r.to(|| HttpResponse::MethodNotAllowed())),
        );

        let req = TestRequest::with_uri("/non-exist").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/app1/non-exist").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/app2/non-exist").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    fn md<S, P, B>(
        req: ServiceRequest<P>,
        srv: &mut S,
    ) -> impl IntoFuture<Item = ServiceResponse<B>, Error = Error>
    where
        S: Service<
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B>,
            Error = Error,
        >,
    {
        srv.call(req).map(|mut res| {
            res.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static("0001"));
            res
        })
    }

    #[test]
    fn test_middleware() {
        let mut srv =
            init_service(App::new().service(web::scope("app").wrap(md).service(
                web::resource("/test").route(web::get().to(|| HttpResponse::Ok())),
            )));
        let req = TestRequest::with_uri("/app/test").to_request();
        let resp = call_success(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[test]
    fn test_middleware_fn() {
        let mut srv = init_service(
            App::new().service(
                web::scope("app")
                    .wrap_fn(|req, srv| {
                        srv.call(req).map(|mut res| {
                            res.headers_mut().insert(
                                header::CONTENT_TYPE,
                                HeaderValue::from_static("0001"),
                            );
                            res
                        })
                    })
                    .route("/test", web::get().to(|| HttpResponse::Ok())),
            ),
        );
        let req = TestRequest::with_uri("/app/test").to_request();
        let resp = call_success(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }
}
