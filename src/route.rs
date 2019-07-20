use std::rc::Rc;

use actix_http::{http::Method, Error};
use actix_service::{NewService, Service};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, IntoFuture, Poll};

use crate::extract::FromRequest;
use crate::guard::{self, Guard};
use crate::handler::{AsyncFactory, AsyncHandler, Extract, Factory, Handler};
use crate::responder::Responder;
use crate::service::{ServiceRequest, ServiceResponse};
use crate::HttpResponse;

type BoxedRouteService<Req, Res> = Box<
    dyn Service<
        Request = Req,
        Response = Res,
        Error = Error,
        Future = Either<
            FutureResult<Res, Error>,
            Box<dyn Future<Item = Res, Error = Error>>,
        >,
    >,
>;

type BoxedRouteNewService<Req, Res> = Box<
    dyn NewService<
        Config = (),
        Request = Req,
        Response = Res,
        Error = Error,
        InitError = (),
        Service = BoxedRouteService<Req, Res>,
        Future = Box<dyn Future<Item = BoxedRouteService<Req, Res>, Error = ()>>,
    >,
>;

/// Resource route definition
///
/// Route uses builder-like pattern for configuration.
/// If handler is not explicitly set, default *404 Not Found* handler is used.
pub struct Route {
    service: BoxedRouteNewService<ServiceRequest, ServiceResponse>,
    guards: Rc<Vec<Box<dyn Guard>>>,
}

impl Route {
    /// Create new route which matches any request.
    pub fn new() -> Route {
        Route {
            service: Box::new(RouteNewService::new(Extract::new(Handler::new(|| {
                HttpResponse::NotFound()
            })))),
            guards: Rc::new(Vec::new()),
        }
    }

    pub(crate) fn take_guards(&mut self) -> Vec<Box<dyn Guard>> {
        std::mem::replace(Rc::get_mut(&mut self.guards).unwrap(), Vec::new())
    }
}

impl NewService for Route {
    type Config = ();
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = RouteService;
    type Future = CreateRouteService;

    fn new_service(&self, _: &()) -> Self::Future {
        CreateRouteService {
            fut: self.service.new_service(&()),
            guards: self.guards.clone(),
        }
    }
}

type RouteFuture = Box<
    dyn Future<Item = BoxedRouteService<ServiceRequest, ServiceResponse>, Error = ()>,
>;

pub struct CreateRouteService {
    fut: RouteFuture,
    guards: Rc<Vec<Box<dyn Guard>>>,
}

impl Future for CreateRouteService {
    type Item = RouteService;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::Ready(service) => Ok(Async::Ready(RouteService {
                service,
                guards: self.guards.clone(),
            })),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

pub struct RouteService {
    service: BoxedRouteService<ServiceRequest, ServiceResponse>,
    guards: Rc<Vec<Box<dyn Guard>>>,
}

impl RouteService {
    pub fn check(&self, req: &mut ServiceRequest) -> bool {
        for f in self.guards.iter() {
            if !f.check(req.head()) {
                return false;
            }
        }
        true
    }
}

impl Service for RouteService {
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = Either<
        FutureResult<Self::Response, Self::Error>,
        Box<dyn Future<Item = Self::Response, Error = Self::Error>>,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        self.service.call(req)
    }
}

impl Route {
    /// Add method guard to the route.
    ///
    /// ```rust
    /// # use actix_web::*;
    /// # fn main() {
    /// App::new().service(web::resource("/path").route(
    ///     web::get()
    ///         .method(http::Method::CONNECT)
    ///         .guard(guard::Header("content-type", "text/plain"))
    ///         .to(|req: HttpRequest| HttpResponse::Ok()))
    /// );
    /// # }
    /// ```
    pub fn method(mut self, method: Method) -> Self {
        Rc::get_mut(&mut self.guards)
            .unwrap()
            .push(Box::new(guard::Method(method)));
        self
    }

    /// Add guard to the route.
    ///
    /// ```rust
    /// # use actix_web::*;
    /// # fn main() {
    /// App::new().service(web::resource("/path").route(
    ///     web::route()
    ///         .guard(guard::Get())
    ///         .guard(guard::Header("content-type", "text/plain"))
    ///         .to(|req: HttpRequest| HttpResponse::Ok()))
    /// );
    /// # }
    /// ```
    pub fn guard<F: Guard + 'static>(mut self, f: F) -> Self {
        Rc::get_mut(&mut self.guards).unwrap().push(Box::new(f));
        self
    }

    /// Set handler function, use request extractors for parameters.
    ///
    /// ```rust
    /// #[macro_use] extern crate serde_derive;
    /// use actix_web::{web, http, App};
    ///
    /// #[derive(Deserialize)]
    /// struct Info {
    ///     username: String,
    /// }
    ///
    /// /// extract path info using serde
    /// fn index(info: web::Path<Info>) -> String {
    ///     format!("Welcome {}!", info.username)
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::resource("/{username}/index.html") // <- define path parameters
    ///             .route(web::get().to(index))        // <- register handler
    ///     );
    /// }
    /// ```
    ///
    /// It is possible to use multiple extractors for one handler function.
    ///
    /// ```rust
    /// # use std::collections::HashMap;
    /// # use serde_derive::Deserialize;
    /// use actix_web::{web, App};
    ///
    /// #[derive(Deserialize)]
    /// struct Info {
    ///     username: String,
    /// }
    ///
    /// /// extract path info using serde
    /// fn index(path: web::Path<Info>, query: web::Query<HashMap<String, String>>, body: web::Json<Info>) -> String {
    ///     format!("Welcome {}!", path.username)
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::resource("/{username}/index.html") // <- define path parameters
    ///             .route(web::get().to(index))
    ///     );
    /// }
    /// ```
    pub fn to<F, T, R>(mut self, handler: F) -> Route
    where
        F: Factory<T, R> + 'static,
        T: FromRequest + 'static,
        R: Responder + 'static,
    {
        self.service =
            Box::new(RouteNewService::new(Extract::new(Handler::new(handler))));
        self
    }

    /// Set async handler function, use request extractors for parameters.
    /// This method has to be used if your handler function returns `impl Future<>`
    ///
    /// ```rust
    /// # use futures::future::ok;
    /// #[macro_use] extern crate serde_derive;
    /// use actix_web::{web, App, Error};
    /// use futures::Future;
    ///
    /// #[derive(Deserialize)]
    /// struct Info {
    ///     username: String,
    /// }
    ///
    /// /// extract path info using serde
    /// fn index(info: web::Path<Info>) -> impl Future<Item = &'static str, Error = Error> {
    ///     ok("Hello World!")
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::resource("/{username}/index.html") // <- define path parameters
    ///             .route(web::get().to_async(index))  // <- register async handler
    ///     );
    /// }
    /// ```
    #[allow(clippy::wrong_self_convention)]
    pub fn to_async<F, T, R>(mut self, handler: F) -> Self
    where
        F: AsyncFactory<T, R>,
        T: FromRequest + 'static,
        R: IntoFuture + 'static,
        R::Item: Responder,
        R::Error: Into<Error>,
    {
        self.service = Box::new(RouteNewService::new(Extract::new(AsyncHandler::new(
            handler,
        ))));
        self
    }
}

struct RouteNewService<T>
where
    T: NewService<Request = ServiceRequest, Error = (Error, ServiceRequest)>,
{
    service: T,
}

impl<T> RouteNewService<T>
where
    T: NewService<
        Config = (),
        Request = ServiceRequest,
        Response = ServiceResponse,
        Error = (Error, ServiceRequest),
    >,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service>::Future: 'static,
{
    pub fn new(service: T) -> Self {
        RouteNewService { service }
    }
}

impl<T> NewService for RouteNewService<T>
where
    T: NewService<
        Config = (),
        Request = ServiceRequest,
        Response = ServiceResponse,
        Error = (Error, ServiceRequest),
    >,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service>::Future: 'static,
{
    type Config = ();
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = BoxedRouteService<ServiceRequest, Self::Response>;
    type Future = Box<dyn Future<Item = Self::Service, Error = Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        Box::new(
            self.service
                .new_service(&())
                .map_err(|_| ())
                .and_then(|service| {
                    let service: BoxedRouteService<_, _> =
                        Box::new(RouteServiceWrapper { service });
                    Ok(service)
                }),
        )
    }
}

struct RouteServiceWrapper<T: Service> {
    service: T,
}

impl<T> Service for RouteServiceWrapper<T>
where
    T::Future: 'static,
    T: Service<
        Request = ServiceRequest,
        Response = ServiceResponse,
        Error = (Error, ServiceRequest),
    >,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = Either<
        FutureResult<Self::Response, Self::Error>,
        Box<dyn Future<Item = Self::Response, Error = Self::Error>>,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(|(e, _)| e)
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        let mut fut = self.service.call(req);
        match fut.poll() {
            Ok(Async::Ready(res)) => Either::A(ok(res)),
            Err((e, req)) => Either::A(ok(req.error_response(e))),
            Ok(Async::NotReady) => Either::B(Box::new(fut.then(|res| match res {
                Ok(res) => Ok(res),
                Err((err, req)) => Ok(req.error_response(err)),
            }))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use futures::Future;
    use serde_derive::Serialize;
    use tokio_timer::sleep;

    use crate::http::{Method, StatusCode};
    use crate::test::{call_service, init_service, read_body, TestRequest};
    use crate::{error, web, App, HttpResponse};

    #[derive(Serialize, PartialEq, Debug)]
    struct MyObject {
        name: String,
    }

    #[test]
    fn test_route() {
        let mut srv = init_service(
            App::new()
                .service(
                    web::resource("/test")
                        .route(web::get().to(|| HttpResponse::Ok()))
                        .route(web::put().to(|| {
                            Err::<HttpResponse, _>(error::ErrorBadRequest("err"))
                        }))
                        .route(web::post().to_async(|| {
                            sleep(Duration::from_millis(100))
                                .then(|_| HttpResponse::Created())
                        }))
                        .route(web::delete().to_async(|| {
                            sleep(Duration::from_millis(100)).then(|_| {
                                Err::<HttpResponse, _>(error::ErrorBadRequest("err"))
                            })
                        })),
                )
                .service(web::resource("/json").route(web::get().to_async(|| {
                    sleep(Duration::from_millis(25)).then(|_| {
                        Ok::<_, crate::Error>(web::Json(MyObject {
                            name: "test".to_string(),
                        }))
                    })
                }))),
        );

        let req = TestRequest::with_uri("/test")
            .method(Method::GET)
            .to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test")
            .method(Method::POST)
            .to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/test")
            .method(Method::PUT)
            .to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/test")
            .method(Method::DELETE)
            .to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/test")
            .method(Method::HEAD)
            .to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/json").to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);

        let body = read_body(resp);
        assert_eq!(body, Bytes::from_static(b"{\"name\":\"test\"}"));
    }
}
