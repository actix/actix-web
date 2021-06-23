use std::fmt;
use std::future::Future;
use std::rc::Rc;

use actix_http::Extensions;
use actix_router::ResourceDef;
use actix_service::boxed::{self, BoxServiceFactory};
use actix_service::{
    apply, apply_fn_factory, IntoServiceFactory, Service, ServiceFactory, ServiceFactoryExt,
    Transform,
};

use crate::config::ServiceConfig;
use crate::data::Data;
use crate::dev::{AppService, HttpServiceFactory};
use crate::error::Error;
use crate::guard::Guard;
use crate::resource::Resource;
use crate::rmap::{self, ResourceMap};
use crate::route::Route;
use crate::service::{
    AppServiceFactory, ServiceFactoryWrapper, ServiceRequest, ServiceResponse,
};

type BoxedFactory = BoxServiceFactory<(), ServiceRequest, ServiceResponse, Error, ()>;

pub trait EndpointConstructor {
    type Output: ServiceFactory<
            ServiceRequest,
            Config = (),
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        > + 'static;

    fn call(&self, f: BoxedFactory) -> Self::Output;
}

impl EndpointConstructor for () {
    type Output = BoxedFactory;

    fn call(&self, f: BoxedFactory) -> Self::Output {
        f
    }
}

impl<F, O> EndpointConstructor for F
where
    F: Fn(BoxedFactory) -> O,
    O: ServiceFactory<
            ServiceRequest,
            Config = (),
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        > + 'static,
{
    type Output = O;

    fn call(&self, f: BoxedFactory) -> Self::Output {
        self(f)
    }
}

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
/// ```
/// use actix_web::{web, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::scope("/{project_id}/")
///             .service(web::resource("/path1").to(|| async { HttpResponse::Ok() }))
///             .service(web::resource("/path2").route(web::get().to(|| HttpResponse::Ok())))
///             .service(web::resource("/path3").route(web::head().to(|| HttpResponse::MethodNotAllowed())))
///     );
/// }
/// ```
///
/// In the above example three routes get registered:
///  * /{project_id}/path1 - responds to all http method
///  * /{project_id}/path2 - `GET` requests
///  * /{project_id}/path3 - `HEAD` requests
// use actix_service::{boxed, IntoServiceFactory, ServiceFactory};
pub struct Scope<Cons = ()> {
    constructor: Cons,
    prefix: String,
    app_data: Option<Extensions>,
    services: Vec<Box<dyn AppServiceFactory>>,
    guards: Vec<Rc<dyn Guard>>,
    default: Option<BoxedFactory>,
    external: Vec<ResourceDef>,
}

impl Scope {
    /// Create a new scope
    pub fn new(path: &str) -> Scope {
        Scope {
            constructor: (),
            prefix: path.to_string(),
            app_data: None,
            guards: Vec::new(),
            services: Vec::new(),
            default: None,
            external: Vec::new(),
        }
    }
}

impl<C: EndpointConstructor> Scope<C> {
    /// Add match guard to a scope.
    ///
    /// ```
    /// use actix_web::{web, guard, App, HttpRequest, HttpResponse};
    ///
    /// async fn index(data: web::Path<(String, String)>) -> &'static str {
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
        self.guards.push(Rc::new(guard));
        self
    }

    /// Set or override application data. Application data could be accessed
    /// by using `Data<T>` extractor where `T` is data type.
    ///
    /// ```
    /// use std::cell::Cell;
    /// use actix_web::{web, App, HttpResponse, Responder};
    ///
    /// struct MyData {
    ///     counter: Cell<usize>,
    /// }
    ///
    /// async fn index(data: web::Data<MyData>) -> impl Responder {
    ///     data.counter.set(data.counter.get() + 1);
    ///     HttpResponse::Ok()
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::scope("/app")
    ///             .data(MyData{ counter: Cell::new(0) })
    ///             .service(
    ///                 web::resource("/index.html").route(
    ///                     web::get().to(index)))
    ///     );
    /// }
    /// ```
    #[deprecated(since = "4.0.0", note = "Use `.app_data(Data::new(val))` instead.")]
    pub fn data<U: 'static>(self, data: U) -> Self {
        self.app_data(Data::new(data))
    }

    /// Add scope data.
    ///
    /// Data of different types from parent contexts will still be accessible.
    pub fn app_data<U: 'static>(mut self, data: U) -> Self {
        self.app_data
            .get_or_insert_with(Extensions::new)
            .insert(data);

        self
    }

    /// Run external configuration as part of the scope building
    /// process
    ///
    /// This function is useful for moving parts of configuration to a
    /// different module or even library. For example,
    /// some of the resource's configuration could be moved to different module.
    ///
    /// ```
    /// # extern crate actix_web;
    /// use actix_web::{web, middleware, App, HttpResponse};
    ///
    /// // this function could be located in different module
    /// fn config(cfg: &mut web::ServiceConfig) {
    ///     cfg.service(web::resource("/test")
    ///         .route(web::get().to(|| HttpResponse::Ok()))
    ///         .route(web::head().to(|| HttpResponse::MethodNotAllowed()))
    ///     );
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .wrap(middleware::Logger::default())
    ///         .service(
    ///             web::scope("/api")
    ///                 .configure(config)
    ///         )
    ///         .route("/index.html", web::get().to(|| HttpResponse::Ok()));
    /// }
    /// ```
    pub fn configure<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut ServiceConfig),
    {
        let mut cfg = ServiceConfig::new();
        f(&mut cfg);
        self.services.extend(cfg.services);
        self.external.extend(cfg.external);

        self.app_data
            .get_or_insert_with(Extensions::new)
            .extend(cfg.app_data);
        self
    }

    /// Register HTTP service.
    ///
    /// This is similar to `App's` service registration.
    ///
    /// Actix Web provides several services implementations:
    ///
    /// * *Resource* is an entry in resource table which corresponds to requested URL.
    /// * *Scope* is a set of resources with common root path.
    /// * "StaticFiles" is a service for static files support
    ///
    /// ```
    /// use actix_web::{web, App, HttpRequest};
    ///
    /// struct AppState;
    ///
    /// async fn index(req: HttpRequest) -> &'static str {
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
        F: HttpServiceFactory + 'static,
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
    /// ```
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// async fn index(data: web::Path<(String, String)>) -> &'static str {
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
    pub fn route(self, path: &str, mut route: Route) -> Self {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Default service to be used if no matching route could be found.
    ///
    /// If default resource is not registered, app's default resource is being used.
    pub fn default_service<F, U>(mut self, f: F) -> Self
    where
        F: IntoServiceFactory<U, ServiceRequest>,
        U: ServiceFactory<
                ServiceRequest,
                Config = (),
                Response = ServiceResponse,
                Error = Error,
            > + 'static,
        U::InitError: fmt::Debug,
    {
        // create and configure default resource
        self.default = Some(boxed::factory(f.into_factory().map_init_err(|e| {
            log::error!("Can not construct default service: {:?}", e)
        })));

        self
    }

    /// Registers middleware, in the form of a middleware component (type),
    /// that runs during inbound processing in the request
    /// life-cycle (request -> response), modifying request as
    /// necessary, across all requests managed by the *Scope*.  Scope-level
    /// middleware is more limited in what it can modify, relative to Route or
    /// Application level middleware, in that Scope-level middleware can not modify
    /// ServiceResponse.
    ///
    /// Use middleware when you need to read or modify *every* request in some way.
    pub fn wrap<M>(self, mw: M) -> Scope<impl EndpointConstructor>
    where
        M: Transform<
                <C::Output as ServiceFactory<ServiceRequest>>::Service,
                ServiceRequest,
                Response = ServiceResponse,
                Error = Error,
                InitError = (),
            > + 'static,
    {
        let mw = Rc::new(mw);
        let constructor = self.constructor;
        Scope {
            constructor: move |factory| apply(mw.clone(), constructor.call(factory)),
            prefix: self.prefix,
            app_data: self.app_data,
            guards: self.guards,
            services: self.services,
            default: self.default,
            external: self.external,
        }
    }

    /// Registers middleware, in the form of a closure, that runs during inbound
    /// processing in the request life-cycle (request -> response), modifying
    /// request as necessary, across all requests managed by the *Scope*.
    /// Scope-level middleware is more limited in what it can modify, relative
    /// to Route or Application level middleware, in that Scope-level middleware
    /// can not modify ServiceResponse.
    ///
    /// ```
    /// use actix_service::Service;
    /// use actix_web::{web, App};
    /// use actix_web::http::{header::CONTENT_TYPE, HeaderValue};
    ///
    /// async fn index() -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::scope("/app")
    ///             .wrap_fn(|req, srv| {
    ///                 let fut = srv.call(req);
    ///                 async {
    ///                     let mut res = fut.await?;
    ///                     res.headers_mut().insert(
    ///                        CONTENT_TYPE, HeaderValue::from_static("text/plain"),
    ///                     );
    ///                     Ok(res)
    ///                 }
    ///             })
    ///             .route("/index.html", web::get().to(index)));
    /// }
    /// ```
    pub fn wrap_fn<F, R>(self, mw: F) -> Scope<impl EndpointConstructor>
    where
        F: Fn(ServiceRequest, &<C::Output as ServiceFactory<ServiceRequest>>::Service) -> R
            + Clone
            + 'static,

        R: Future<Output = Result<ServiceResponse, Error>>,
    {
        let constructor = self.constructor;
        Scope {
            constructor: move |factory| apply_fn_factory(constructor.call(factory), mw.clone()),
            prefix: self.prefix,
            app_data: self.app_data,
            guards: self.guards,
            services: self.services,
            default: self.default,
            external: self.external,
        }
    }

    fn _register(mut self, config: &mut AppService) {
        let default_service = self.default.take();
        let services = self.services.drain(..).collect::<Vec<_>>();
        let mut external_resources = self.external.drain(..).collect::<Vec<_>>();

        let wrapped_guards = |guards: Option<Vec<Box<dyn Guard>>>| {
            let guards = self
                .guards
                .iter()
                .map(|g| Box::new(g.clone()) as Box<dyn Guard>)
                .chain(guards.into_iter().flatten())
                .collect::<Vec<_>>();
            match guards {
                guards if !guards.is_empty() => Some(guards),
                _ => None,
            }
        };

        let wrapped_rdef = |mut rdef: ResourceDef| {
            rmap::rdef_set_root_prefix(&mut rdef, &self.prefix);
            rdef
        };

        let mut wrapped_rmap = |rmap: Option<Rc<ResourceMap>>, rdef: &ResourceDef| {
            let rmap = rmap.map(|rmap| {
                let mut rmap = (*rmap).clone();
                rmap.set_root_prefix(&self.prefix);
                rmap
            });
            if external_resources.is_empty() {
                rmap
            } else {
                let mut rmap = rmap.unwrap_or_else(|| ResourceMap::new(rdef.clone()));
                for ext in external_resources.iter_mut() {
                    rmap.add(ext, None);
                }
                Some(rmap)
            }
            .map(Rc::new)
        };

        let mut config_dummy = config.clone_config();
        services
            .into_iter()
            .for_each(|mut srv| srv.register(&mut config_dummy));

        let (_, services) = config_dummy.into_services();

        for (rdef, factory, guards, rmap) in services {
            let rdef = wrapped_rdef(rdef);
            let rmap = wrapped_rmap(rmap, &rdef);
            let guards = wrapped_guards(guards);
            let factory = self.constructor.call(factory);
            config.register_service(rdef, guards, factory, rmap);
        }

        if let Some(default) = default_service {
            let root_rdef = ResourceDef::root_prefix(&self.prefix);
            let rmap = wrapped_rmap(None, &root_rdef);
            let guards = wrapped_guards(None);
            let factory = self.constructor.call(default);
            config.register_service(root_rdef, guards, factory, rmap);
        }
    }
}

impl<C: EndpointConstructor> HttpServiceFactory for Scope<C> {
    fn register(mut self, config: &mut AppService) {
        if let Some(app_data) = self.app_data.take().map(Rc::new) {
            return self
                .wrap_fn(move |mut req, srv| {
                    req.add_data_container(app_data.clone());
                    srv.call(req)
                })
                ._register(config);
        } else {
            self._register(config);
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_service::Service;
    use actix_utils::future::ok;
    use bytes::Bytes;

    use crate::dev::Body;
    use crate::http::{header, HeaderValue, Method, StatusCode};
    use crate::middleware::DefaultHeaders;
    use crate::service::ServiceRequest;
    use crate::test::{call_service, init_service, read_body, TestRequest};
    use crate::{guard, web, App, HttpRequest, HttpResponse};

    #[actix_rt::test]
    async fn test_scope() {
        let srv =
            init_service(App::new().service(
                web::scope("/app").service(web::resource("/path1").to(HttpResponse::Ok)),
            ))
            .await;

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_scope_root() {
        let srv = init_service(
            App::new().service(
                web::scope("/app")
                    .service(web::resource("").to(HttpResponse::Ok))
                    .service(web::resource("/").to(HttpResponse::Created)),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[actix_rt::test]
    async fn test_scope_root2() {
        let srv = init_service(
            App::new()
                .service(web::scope("/app/").service(web::resource("").to(HttpResponse::Ok))),
        )
        .await;

        let req = TestRequest::with_uri("/app").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_scope_root3() {
        let srv = init_service(
            App::new()
                .service(web::scope("/app/").service(web::resource("/").to(HttpResponse::Ok))),
        )
        .await;

        let req = TestRequest::with_uri("/app").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_scope_route() {
        let srv = init_service(
            App::new().service(
                web::scope("app")
                    .route("/path1", web::get().to(HttpResponse::Ok))
                    .route("/path1", web::delete().to(HttpResponse::Ok)),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::DELETE)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_scope_route_without_leading_slash() {
        let srv = init_service(
            App::new().service(
                web::scope("app").service(
                    web::resource("path1")
                        .route(web::get().to(HttpResponse::Ok))
                        .route(web::delete().to(HttpResponse::Ok)),
                ),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::DELETE)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[actix_rt::test]
    async fn test_scope_guard() {
        let srv = init_service(
            App::new().service(
                web::scope("/app")
                    .guard(guard::Get())
                    .service(web::resource("/path1").to(HttpResponse::Ok)),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::GET)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_scope_variable_segment() {
        let srv = init_service(App::new().service(web::scope("/ab-{project}").service(
            web::resource("/path1").to(|r: HttpRequest| {
                HttpResponse::Ok().body(format!("project: {}", &r.match_info()["project"]))
            }),
        )))
        .await;

        let req = TestRequest::with_uri("/ab-project1/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        match resp.response().body() {
            Body::Bytes(ref b) => {
                let bytes = b.clone();
                assert_eq!(bytes, Bytes::from_static(b"project: project1"));
            }
            _ => panic!(),
        }

        let req = TestRequest::with_uri("/aa-project1/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_nested_scope() {
        let srv = init_service(App::new().service(web::scope("/app").service(
            web::scope("/t1").service(web::resource("/path1").to(HttpResponse::Created)),
        )))
        .await;

        let req = TestRequest::with_uri("/app/t1/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[actix_rt::test]
    async fn test_nested_scope_no_slash() {
        let srv = init_service(App::new().service(web::scope("/app").service(
            web::scope("t1").service(web::resource("/path1").to(HttpResponse::Created)),
        )))
        .await;

        let req = TestRequest::with_uri("/app/t1/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[actix_rt::test]
    async fn test_nested_scope_root() {
        let srv = init_service(
            App::new().service(
                web::scope("/app").service(
                    web::scope("/t1")
                        .service(web::resource("").to(HttpResponse::Ok))
                        .service(web::resource("/").to(HttpResponse::Created)),
                ),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/t1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/t1/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[actix_rt::test]
    async fn test_nested_scope_filter() {
        let srv = init_service(
            App::new().service(
                web::scope("/app").service(
                    web::scope("/t1")
                        .guard(guard::Get())
                        .service(web::resource("/path1").to(HttpResponse::Ok)),
                ),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/t1/path1")
            .method(Method::POST)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/t1/path1")
            .method(Method::GET)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_nested_scope_with_variable_segment() {
        let srv = init_service(App::new().service(web::scope("/app").service(
            web::scope("/{project_id}").service(web::resource("/path1").to(
                |r: HttpRequest| {
                    HttpResponse::Created()
                        .body(format!("project: {}", &r.match_info()["project_id"]))
                },
            )),
        )))
        .await;

        let req = TestRequest::with_uri("/app/project_1/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        match resp.response().body() {
            Body::Bytes(ref b) => {
                let bytes = b.clone();
                assert_eq!(bytes, Bytes::from_static(b"project: project_1"));
            }
            _ => panic!(),
        }
    }

    #[actix_rt::test]
    async fn test_nested2_scope_with_variable_segment() {
        let srv = init_service(App::new().service(web::scope("/app").service(
            web::scope("/{project}").service(web::scope("/{id}").service(
                web::resource("/path1").to(|r: HttpRequest| {
                    HttpResponse::Created().body(format!(
                        "project: {} - {}",
                        &r.match_info()["project"],
                        &r.match_info()["id"],
                    ))
                }),
            )),
        )))
        .await;

        let req = TestRequest::with_uri("/app/test/1/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        match resp.response().body() {
            Body::Bytes(ref b) => {
                let bytes = b.clone();
                assert_eq!(bytes, Bytes::from_static(b"project: test - 1"));
            }
            _ => panic!(),
        }

        let req = TestRequest::with_uri("/app/test/1/path2").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_default_resource() {
        let srv = init_service(
            App::new().service(
                web::scope("/app")
                    .service(web::resource("/path1").to(HttpResponse::Ok))
                    .default_service(|r: ServiceRequest| {
                        ok(r.into_response(HttpResponse::BadRequest()))
                    }),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/path2").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/path2").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_default_resource_propagation() {
        let srv = init_service(
            App::new()
                .service(
                    web::scope("/app1")
                        .default_service(web::resource("").to(HttpResponse::BadRequest)),
                )
                .service(web::scope("/app2"))
                .default_service(|r: ServiceRequest| {
                    ok(r.into_response(HttpResponse::MethodNotAllowed()))
                }),
        )
        .await;

        let req = TestRequest::with_uri("/non-exist").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/app1/non-exist").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/app2/non-exist").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[actix_rt::test]
    async fn test_middleware() {
        let srv = init_service(
            App::new().service(
                web::scope("app")
                    .wrap(
                        DefaultHeaders::new()
                            .header(header::CONTENT_TYPE, HeaderValue::from_static("0001")),
                    )
                    .service(web::resource("/test").route(web::get().to(HttpResponse::Ok))),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[actix_rt::test]
    async fn test_middleware_fn() {
        let srv = init_service(
            App::new().service(
                web::scope("app")
                    .wrap_fn(|req, srv| {
                        let fut = srv.call(req);
                        async move {
                            let mut res = fut.await?;
                            res.headers_mut()
                                .insert(header::CONTENT_TYPE, HeaderValue::from_static("0001"));
                            Ok(res)
                        }
                    })
                    .route("/test", web::get().to(HttpResponse::Ok)),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    // allow deprecated App::data
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_override_data() {
        let srv = init_service(App::new().data(1usize).service(
            web::scope("app").data(10usize).route(
                "/t",
                web::get().to(|data: web::Data<usize>| {
                    assert_eq!(**data, 10);
                    HttpResponse::Ok()
                }),
            ),
        ))
        .await;

        let req = TestRequest::with_uri("/app/t").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // allow deprecated App::data
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_override_data_default_service() {
        let srv = init_service(App::new().data(1usize).service(
            web::scope("app").data(10usize).default_service(web::to(
                |data: web::Data<usize>| {
                    assert_eq!(**data, 10);
                    HttpResponse::Ok()
                },
            )),
        ))
        .await;

        let req = TestRequest::with_uri("/app/t").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_override_app_data() {
        let srv = init_service(App::new().app_data(web::Data::new(1usize)).service(
            web::scope("app").app_data(web::Data::new(10usize)).route(
                "/t",
                web::get().to(|data: web::Data<usize>| {
                    assert_eq!(**data, 10);
                    HttpResponse::Ok()
                }),
            ),
        ))
        .await;

        let req = TestRequest::with_uri("/app/t").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_scope_config() {
        let srv = init_service(App::new().service(web::scope("/app").configure(|s| {
            s.route("/path1", web::get().to(HttpResponse::Ok));
        })))
        .await;

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_scope_config_2() {
        let srv = init_service(App::new().service(web::scope("/app").configure(|s| {
            s.service(web::scope("/v1").configure(|s| {
                s.route("/", web::get().to(HttpResponse::Ok));
            }));
        })))
        .await;

        let req = TestRequest::with_uri("/app/v1/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_url_for_external() {
        let srv = init_service(App::new().service(web::scope("/app").configure(|s| {
            s.service(web::scope("/v1").configure(|s| {
                s.external_resource("youtube", "https://youtube.com/watch/{video_id}");
                s.route(
                    "/",
                    web::get().to(|req: HttpRequest| {
                        HttpResponse::Ok()
                            .body(req.url_for("youtube", &["xxxxxx"]).unwrap().to_string())
                    }),
                );
            }));
        })))
        .await;

        let req = TestRequest::with_uri("/app/v1/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body(resp).await;
        assert_eq!(body, &b"https://youtube.com/watch/xxxxxx"[..]);
    }

    #[actix_rt::test]
    async fn test_url_for_nested() {
        let srv = init_service(App::new().service(web::scope("/a").service(
            web::scope("/b").service(web::resource("/c/{stuff}").name("c").route(
                web::get().to(|req: HttpRequest| {
                    HttpResponse::Ok()
                        .body(format!("{}", req.url_for("c", &["12345"]).unwrap()))
                }),
            )),
        )))
        .await;

        let req = TestRequest::with_uri("/a/b/c/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body(resp).await;
        assert_eq!(
            body,
            Bytes::from_static(b"http://localhost:8080/a/b/c/12345")
        );
    }
}
