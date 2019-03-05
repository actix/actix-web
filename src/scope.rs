use std::cell::RefCell;
use std::rc::Rc;

use actix_http::Response;
use actix_router::{ResourceDef, ResourceInfo, Router};
use actix_service::boxed::{self, BoxedNewService, BoxedService};
use actix_service::{
    ApplyTransform, IntoNewService, IntoTransform, NewService, Service, Transform,
};
use futures::future::{ok, Either, Future, FutureResult};
use futures::{Async, Poll};

use crate::guard::Guard;
use crate::resource::Resource;
use crate::route::Route;
use crate::service::{ServiceRequest, ServiceResponse};

type Guards = Vec<Box<Guard>>;
type HttpService<P> = BoxedService<ServiceRequest<P>, ServiceResponse, ()>;
type HttpNewService<P> = BoxedNewService<(), ServiceRequest<P>, ServiceResponse, (), ()>;
type BoxedResponse = Box<Future<Item = ServiceResponse, Error = ()>>;

/// Resources scope
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
///     let app = App::new().scope("/{project_id}/", |scope| {
///         scope
///             .resource("/path1", |r| r.to(|| HttpResponse::Ok()))
///             .resource("/path2", |r| r.route(web::get().to(|| HttpResponse::Ok())))
///             .resource("/path3", |r| r.route(web::head().to(|| HttpResponse::MethodNotAllowed())))
///     });
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
    rdef: ResourceDef,
    services: Vec<(ResourceDef, HttpNewService<P>, Option<Guards>)>,
    guards: Vec<Box<Guard>>,
    default: Rc<RefCell<Option<Rc<HttpNewService<P>>>>>,
    defaults: Vec<Rc<RefCell<Option<Rc<HttpNewService<P>>>>>>,
    factory_ref: Rc<RefCell<Option<ScopeFactory<P>>>>,
}

impl<P: 'static> Scope<P> {
    /// Create a new scope
    pub fn new(path: &str) -> Scope<P> {
        let fref = Rc::new(RefCell::new(None));
        let rdef = ResourceDef::prefix(&insert_slash(path));
        Scope {
            endpoint: ScopeEndpoint::new(fref.clone()),
            rdef: rdef.clone(),
            guards: Vec::new(),
            services: Vec::new(),
            default: Rc::new(RefCell::new(None)),
            defaults: Vec::new(),
            factory_ref: fref,
        }
    }
}

impl<P: 'static, T> Scope<P, T>
where
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (),
        InitError = (),
    >,
{
    #[inline]
    pub(crate) fn rdef(&self) -> &ResourceDef {
        &self.rdef
    }

    /// Add guard to a scope.
    ///
    /// ```rust
    /// use actix_web::{web, guard, App, HttpRequest, HttpResponse, extract::Path};
    ///
    /// fn index(data: Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().scope("/app", |scope| {
    ///         scope
    ///             .guard(guard::Header("content-type", "text/plain"))
    ///             .route("/test1",web::get().to(index))
    ///             .route("/test2", web::post().to(|r: HttpRequest| {
    ///                 HttpResponse::MethodNotAllowed()
    ///             }))
    ///     });
    /// }
    /// ```
    pub fn guard<G: Guard + 'static>(mut self, guard: G) -> Self {
        self.guards.push(Box::new(guard));
        self
    }

    /// Create nested scope.
    ///
    /// ```rust
    /// use actix_web::{App, HttpRequest};
    ///
    /// struct AppState;
    ///
    /// fn index(req: HttpRequest) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().scope("/app", |scope| {
    ///         scope.nested("/v1", |scope| scope.resource("/test1", |r| r.to(index)))
    ///     });
    /// }
    /// ```
    pub fn nested<F>(mut self, path: &str, f: F) -> Self
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

    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `Scope::resource()` method.
    /// This method can not be could multiple times, in that case
    /// multiple resources with one route would be registered for same resource path.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpResponse, extract::Path};
    ///
    /// fn index(data: Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().scope("/app", |scope| {
    ///         scope.route("/test1", web::get().to(index))
    ///             .route("/test2", web::post().to(|| HttpResponse::MethodNotAllowed()))
    ///     });
    /// }
    /// ```
    pub fn route(self, path: &str, route: Route<P>) -> Self {
        self.resource(path, move |r| r.route(route))
    }

    /// configure resource for a specific path.
    ///
    /// This method is similar to an `App::resource()` method.
    /// Resources may have variable path segments. Resource path uses scope
    /// path as a path prefix.
    ///
    /// ```rust
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let app = App::new().scope("/api", |scope| {
    ///         scope.resource("/users/{userid}/{friend}", |r| {
    ///             r.route(web::get().to(|| HttpResponse::Ok()))
    ///              .route(web::head().to(|| HttpResponse::MethodNotAllowed()))
    ///              .route(web::route()
    ///                 .guard(guard::Any(guard::Get()).or(guard::Put()))
    ///                 .guard(guard::Header("Content-Type", "text/plain"))
    ///                 .to(|| HttpResponse::Ok()))
    ///         })
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
        // add resource
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
    pub fn default_resource<F, U>(mut self, f: F) -> Self
    where
        F: FnOnce(Resource<P>) -> Resource<P, U>,
        U: NewService<
                Request = ServiceRequest<P>,
                Response = ServiceResponse,
                Error = (),
                InitError = (),
            > + 'static,
    {
        // create and configure default resource
        self.default = Rc::new(RefCell::new(Some(Rc::new(boxed::new_service(
            f(Resource::new()).into_new_service().map_init_err(|_| ()),
        )))));

        self
    }

    /// Register a scope middleware
    ///
    /// This is similar to `App's` middlewares, but
    /// middleware is not allowed to change response type (i.e modify response's body).
    /// Middleware get invoked on scope level.
    pub fn middleware<M, F>(
        self,
        mw: F,
    ) -> Scope<
        P,
        impl NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse,
            Error = (),
            InitError = (),
        >,
    >
    where
        M: Transform<
            T::Service,
            Request = ServiceRequest<P>,
            Response = ServiceResponse,
            Error = (),
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
            defaults: self.defaults,
            factory_ref: self.factory_ref,
        }
    }

    pub(crate) fn get_default(&self) -> Rc<RefCell<Option<Rc<HttpNewService<P>>>>> {
        self.default.clone()
    }

    pub(crate) fn take_guards(&mut self) -> Option<Vec<Box<Guard>>> {
        if self.guards.is_empty() {
            None
        } else {
            Some(std::mem::replace(&mut self.guards, Vec::new()))
        }
    }
}

pub(crate) fn insert_slash(path: &str) -> String {
    let mut path = path.to_owned();
    if !path.is_empty() && !path.starts_with('/') {
        path.insert(0, '/');
    };
    path
}

impl<P, T> IntoNewService<T> for Scope<P, T>
where
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (),
        InitError = (),
    >,
{
    fn into_new_service(self) -> T {
        // update resource default service
        if let Some(ref d) = *self.default.as_ref().borrow() {
            for default in &self.defaults {
                if default.borrow_mut().is_none() {
                    *default.borrow_mut() = Some(d.clone());
                }
            }
        }

        *self.factory_ref.borrow_mut() = Some(ScopeFactory {
            default: self.default.clone(),
            services: Rc::new(
                self.services
                    .into_iter()
                    .map(|(rdef, srv, guards)| (rdef, srv, RefCell::new(guards)))
                    .collect(),
            ),
        });

        self.endpoint
    }
}

pub struct ScopeFactory<P> {
    services: Rc<Vec<(ResourceDef, HttpNewService<P>, RefCell<Option<Guards>>)>>,
    default: Rc<RefCell<Option<Rc<HttpNewService<P>>>>>,
}

impl<P: 'static> NewService for ScopeFactory<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
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

/// Create app service
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
                            router.rdef(path, service);
                            router.set_user_data(guards);
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
    type Error = ();
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
            let req = req.into_request();
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
    type Error = ();
    type InitError = ();
    type Service = ScopeService<P>;
    type Future = ScopeFactoryResponse<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        self.factory.borrow_mut().as_mut().unwrap().new_service(&())
    }
}

#[cfg(test)]
mod tests {
    use actix_http::body::{Body, ResponseBody};
    use actix_http::http::{Method, StatusCode};
    use actix_service::{IntoNewService, NewService, Service};
    use bytes::Bytes;

    use crate::test::{block_on, TestRequest};
    use crate::{guard, web, App, HttpRequest, HttpResponse};

    #[test]
    fn test_scope() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.resource("/path1", |r| r.to(|| HttpResponse::Ok()))
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_scope_root() {
        let app = App::new()
            .scope("/app", |scope| {
                scope
                    .resource("", |r| r.to(|| HttpResponse::Ok()))
                    .resource("/", |r| r.to(|| HttpResponse::Created()))
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/app").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_scope_root2() {
        let app = App::new()
            .scope("/app/", |scope| {
                scope.resource("", |r| r.to(|| HttpResponse::Ok()))
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/app").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_scope_root3() {
        let app = App::new()
            .scope("/app/", |scope| {
                scope.resource("/", |r| r.to(|| HttpResponse::Ok()))
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/app").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_scope_route() {
        let app = App::new()
            .scope("app", |scope| {
                scope.resource("/path1", |r| {
                    r.route(web::get().to(|| HttpResponse::Ok()))
                        .route(web::delete().to(|| HttpResponse::Ok()))
                })
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

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
        let app = App::new()
            .scope("app", |scope| {
                scope.resource("path1", |r| {
                    r.route(web::get().to(|| HttpResponse::Ok()))
                        .route(web::delete().to(|| HttpResponse::Ok()))
                })
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

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
    fn test_scope_guard() {
        let app = App::new()
            .scope("/app", |scope| {
                scope
                    .guard(guard::Get())
                    .resource("/path1", |r| r.to(|| HttpResponse::Ok()))
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

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
        let app = App::new()
            .scope("/ab-{project}", |scope| {
                scope.resource("/path1", |r| {
                    r.to(|r: HttpRequest| {
                        HttpResponse::Ok()
                            .body(format!("project: {}", &r.match_info()["project"]))
                    })
                })
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/ab-project1/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        match resp.body() {
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
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("/t1", |scope| {
                    scope.resource("/path1", |r| r.to(|| HttpResponse::Created()))
                })
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/app/t1/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_nested_scope_no_slash() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("t1", |scope| {
                    scope.resource("/path1", |r| r.to(|| HttpResponse::Created()))
                })
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/app/t1/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_nested_scope_root() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("/t1", |scope| {
                    scope
                        .resource("", |r| r.to(|| HttpResponse::Ok()))
                        .resource("/", |r| r.to(|| HttpResponse::Created()))
                })
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/app/t1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/t1/").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_nested_scope_filter() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("/t1", |scope| {
                    scope
                        .guard(guard::Get())
                        .resource("/path1", |r| r.to(|| HttpResponse::Ok()))
                })
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

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
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("/{project_id}", |scope| {
                    scope.resource("/path1", |r| {
                        r.to(|r: HttpRequest| {
                            HttpResponse::Created().body(format!(
                                "project: {}",
                                &r.match_info()["project_id"]
                            ))
                        })
                    })
                })
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/app/project_1/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        match resp.body() {
            ResponseBody::Body(Body::Bytes(ref b)) => {
                let bytes: Bytes = b.clone().into();
                assert_eq!(bytes, Bytes::from_static(b"project: project_1"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_nested2_scope_with_variable_segment() {
        let app = App::new()
            .scope("/app", |scope| {
                scope.nested("/{project}", |scope| {
                    scope.nested("/{id}", |scope| {
                        scope.resource("/path1", |r| {
                            r.to(|r: HttpRequest| {
                                HttpResponse::Created().body(format!(
                                    "project: {} - {}",
                                    &r.match_info()["project"],
                                    &r.match_info()["id"],
                                ))
                            })
                        })
                    })
                })
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/app/test/1/path1").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        match resp.body() {
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
        let app = App::new()
            .scope("/app", |scope| {
                scope
                    .resource("/path1", |r| r.to(|| HttpResponse::Ok()))
                    .default_resource(|r| r.to(|| HttpResponse::BadRequest()))
            })
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

        let req = TestRequest::with_uri("/app/path2").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/path2").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_default_resource_propagation() {
        let app = App::new()
            .scope("/app1", |scope| {
                scope.default_resource(|r| r.to(|| HttpResponse::BadRequest()))
            })
            .scope("/app2", |scope| scope)
            .default_resource(|r| r.to(|| HttpResponse::MethodNotAllowed()))
            .into_new_service();
        let mut srv = block_on(app.new_service(&())).unwrap();

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
}
