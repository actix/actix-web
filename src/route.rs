use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::{http::Method, Error, Extensions, Response};
use actix_service::{NewService, Service};
use futures::{Async, Future, IntoFuture, Poll};

use crate::extract::FromRequest;
use crate::guard::{self, Guard};
use crate::handler::{AsyncFactory, AsyncHandler, Extract, Factory, Handler};
use crate::responder::Responder;
use crate::service::{ServiceFromRequest, ServiceRequest, ServiceResponse};
use crate::HttpResponse;

type BoxedRouteService<Req, Res> = Box<
    Service<
        Request = Req,
        Response = Res,
        Error = Error,
        Future = Box<Future<Item = Res, Error = Error>>,
    >,
>;

type BoxedRouteNewService<Req, Res> = Box<
    NewService<
        Request = Req,
        Response = Res,
        Error = Error,
        InitError = (),
        Service = BoxedRouteService<Req, Res>,
        Future = Box<Future<Item = BoxedRouteService<Req, Res>, Error = ()>>,
    >,
>;

/// Resource route definition
///
/// Route uses builder-like pattern for configuration.
/// If handler is not explicitly set, default *404 Not Found* handler is used.
pub struct Route<P> {
    service: BoxedRouteNewService<ServiceRequest<P>, ServiceResponse>,
    guards: Rc<Vec<Box<Guard>>>,
    data: Option<Extensions>,
    data_ref: Rc<RefCell<Option<Rc<Extensions>>>>,
}

impl<P: 'static> Route<P> {
    /// Create new route which matches any request.
    pub fn new() -> Route<P> {
        let data_ref = Rc::new(RefCell::new(None));
        Route {
            service: Box::new(RouteNewService::new(
                Extract::new(data_ref.clone()).and_then(
                    Handler::new(HttpResponse::NotFound).map_err(|_| panic!()),
                ),
            )),
            guards: Rc::new(Vec::new()),
            data: None,
            data_ref,
        }
    }

    pub(crate) fn finish(mut self) -> Self {
        *self.data_ref.borrow_mut() = self.data.take().map(|e| Rc::new(e));
        self
    }

    pub(crate) fn take_guards(&mut self) -> Vec<Box<Guard>> {
        std::mem::replace(Rc::get_mut(&mut self.guards).unwrap(), Vec::new())
    }
}

impl<P> NewService for Route<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = RouteService<P>;
    type Future = CreateRouteService<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        CreateRouteService {
            fut: self.service.new_service(&()),
            guards: self.guards.clone(),
        }
    }
}

type RouteFuture<P> = Box<
    Future<Item = BoxedRouteService<ServiceRequest<P>, ServiceResponse>, Error = ()>,
>;

pub struct CreateRouteService<P> {
    fut: RouteFuture<P>,
    guards: Rc<Vec<Box<Guard>>>,
}

impl<P> Future for CreateRouteService<P> {
    type Item = RouteService<P>;
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

pub struct RouteService<P> {
    service: BoxedRouteService<ServiceRequest<P>, ServiceResponse>,
    guards: Rc<Vec<Box<Guard>>>,
}

impl<P> RouteService<P> {
    pub fn check(&self, req: &mut ServiceRequest<P>) -> bool {
        for f in self.guards.iter() {
            if !f.check(req.head()) {
                return false;
            }
        }
        true
    }
}

impl<P> Service for RouteService<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: ServiceRequest<P>) -> Self::Future {
        self.service.call(req)
    }
}

impl<P: 'static> Route<P> {
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
    pub fn to<F, T, R>(mut self, handler: F) -> Route<P>
    where
        F: Factory<T, R> + 'static,
        T: FromRequest<P> + 'static,
        R: Responder + 'static,
    {
        self.service = Box::new(RouteNewService::new(
            Extract::new(self.data_ref.clone())
                .and_then(Handler::new(handler).map_err(|_| panic!())),
        ));
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
        T: FromRequest<P> + 'static,
        R: IntoFuture + 'static,
        R::Item: Into<Response>,
        R::Error: Into<Error>,
    {
        self.service = Box::new(RouteNewService::new(
            Extract::new(self.data_ref.clone())
                .and_then(AsyncHandler::new(handler).map_err(|_| panic!())),
        ));
        self
    }

    /// Provide route specific data. This method allows to add extractor
    /// configuration or specific state available via `RouteData<T>` extractor.
    ///
    /// ```rust
    /// use actix_web::{web, App};
    ///
    /// /// extract text data from request
    /// fn index(body: String) -> String {
    ///     format!("Body {}!", body)
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::resource("/index.html").route(
    ///             web::get()
    ///                // limit size of the payload
    ///                .data(web::PayloadConfig::new(4096))
    ///                // register handler
    ///                .to(index)
    ///         ));
    /// }
    /// ```
    pub fn data<C: 'static>(mut self, data: C) -> Self {
        if self.data.is_none() {
            self.data = Some(Extensions::new());
        }
        self.data.as_mut().unwrap().insert(data);
        self
    }
}

struct RouteNewService<P, T>
where
    T: NewService<Request = ServiceRequest<P>, Error = (Error, ServiceFromRequest<P>)>,
{
    service: T,
    _t: PhantomData<P>,
}

impl<P: 'static, T> RouteNewService<P, T>
where
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (Error, ServiceFromRequest<P>),
    >,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service>::Future: 'static,
{
    pub fn new(service: T) -> Self {
        RouteNewService {
            service,
            _t: PhantomData,
        }
    }
}

impl<P: 'static, T> NewService for RouteNewService<P, T>
where
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (Error, ServiceFromRequest<P>),
    >,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service>::Future: 'static,
{
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = BoxedRouteService<ServiceRequest<P>, Self::Response>;
    type Future = Box<Future<Item = Self::Service, Error = Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        Box::new(
            self.service
                .new_service(&())
                .map_err(|_| ())
                .and_then(|service| {
                    let service: BoxedRouteService<_, _> =
                        Box::new(RouteServiceWrapper {
                            service,
                            _t: PhantomData,
                        });
                    Ok(service)
                }),
        )
    }
}

struct RouteServiceWrapper<P, T: Service> {
    service: T,
    _t: PhantomData<P>,
}

impl<P, T> Service for RouteServiceWrapper<P, T>
where
    T::Future: 'static,
    T: Service<
        Request = ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (Error, ServiceFromRequest<P>),
    >,
{
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(|(e, _)| e)
    }

    fn call(&mut self, req: ServiceRequest<P>) -> Self::Future {
        Box::new(self.service.call(req).then(|res| match res {
            Ok(res) => Ok(res),
            Err((err, req)) => Ok(req.error_response(err)),
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::http::{Method, StatusCode};
    use crate::test::{call_success, init_service, TestRequest};
    use crate::{web, App, Error, HttpResponse};

    #[test]
    fn test_route() {
        let mut srv = init_service(
            App::new().service(
                web::resource("/test")
                    .route(web::get().to(|| HttpResponse::Ok()))
                    .route(
                        web::post().to_async(|| Ok::<_, Error>(HttpResponse::Created())),
                    ),
            ),
        );

        let req = TestRequest::with_uri("/test")
            .method(Method::GET)
            .to_request();
        let resp = call_success(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test")
            .method(Method::POST)
            .to_request();
        let resp = call_success(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/test")
            .method(Method::HEAD)
            .to_request();
        let resp = call_success(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
