#![allow(clippy::rc_buffer)] // inner value is mutated before being shared (`Rc::get_mut`)

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use actix_http::{http::Method, Error};
use actix_service::{Service, ServiceFactory};
use futures_util::future::{ready, FutureExt, LocalBoxFuture};

use crate::extract::FromRequest;
use crate::guard::{self, Guard};
use crate::handler::{Handler, HandlerService};
use crate::responder::Responder;
use crate::service::{ServiceRequest, ServiceResponse};
use crate::HttpResponse;

type BoxedRouteService = Box<
    dyn Service<
        ServiceRequest,
        Response = ServiceResponse,
        Error = Error,
        Future = LocalBoxFuture<'static, Result<ServiceResponse, Error>>,
    >,
>;

type BoxedRouteNewService = Box<
    dyn ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse,
        Error = Error,
        InitError = (),
        Service = BoxedRouteService,
        Future = LocalBoxFuture<'static, Result<BoxedRouteService, ()>>,
    >,
>;

/// Resource route definition
///
/// Route uses builder-like pattern for configuration.
/// If handler is not explicitly set, default *404 Not Found* handler is used.
pub struct Route {
    service: BoxedRouteNewService,
    guards: Rc<Vec<Box<dyn Guard>>>,
}

impl Route {
    /// Create new route which matches any request.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Route {
        Route {
            service: Box::new(RouteNewService::new(HandlerService::new(|| {
                ready(HttpResponse::NotFound())
            }))),
            guards: Rc::new(Vec::new()),
        }
    }

    pub(crate) fn take_guards(&mut self) -> Vec<Box<dyn Guard>> {
        std::mem::take(Rc::get_mut(&mut self.guards).unwrap())
    }
}

impl ServiceFactory<ServiceRequest> for Route {
    type Config = ();
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = RouteService;
    type Future = CreateRouteService;

    fn new_service(&self, _: ()) -> Self::Future {
        CreateRouteService {
            fut: self.service.new_service(()),
            guards: self.guards.clone(),
        }
    }
}

pub struct CreateRouteService {
    fut: LocalBoxFuture<'static, Result<BoxedRouteService, ()>>,
    guards: Rc<Vec<Box<dyn Guard>>>,
}

impl Future for CreateRouteService {
    type Output = Result<RouteService, ()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match this.fut.as_mut().poll(cx)? {
            Poll::Ready(service) => Poll::Ready(Ok(RouteService {
                service,
                guards: this.guards.clone(),
            })),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub struct RouteService {
    service: BoxedRouteService,
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

impl Service<ServiceRequest> for RouteService {
    type Response = ServiceResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
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
    /// use actix_web::{web, http, App};
    /// use serde_derive::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct Info {
    ///     username: String,
    /// }
    ///
    /// /// extract path info using serde
    /// async fn index(info: web::Path<Info>) -> String {
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
    /// async fn index(path: web::Path<Info>, query: web::Query<HashMap<String, String>>, body: web::Json<Info>) -> String {
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
    pub fn to<F, T, R>(mut self, handler: F) -> Self
    where
        F: Handler<T, R>,
        T: FromRequest + 'static,
        R: Future + 'static,
        R::Output: Responder + 'static,
    {
        self.service = Box::new(RouteNewService::new(HandlerService::new(handler)));
        self
    }
}

struct RouteNewService<T>
where
    T: ServiceFactory<ServiceRequest, Error = Error>,
{
    service: T,
}

impl<T> RouteNewService<T>
where
    T: ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse,
        Error = Error,
    >,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service<ServiceRequest>>::Future: 'static,
{
    pub fn new(service: T) -> Self {
        RouteNewService { service }
    }
}

impl<T> ServiceFactory<ServiceRequest> for RouteNewService<T>
where
    T: ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse,
        Error = Error,
    >,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service<ServiceRequest>>::Future: 'static,
{
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = BoxedRouteService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        self.service
            .new_service(())
            .map(|result| match result {
                Ok(service) => {
                    let service = Box::new(RouteServiceWrapper { service }) as _;
                    Ok(service)
                }
                Err(_) => Err(()),
            })
            .boxed_local()
    }
}

struct RouteServiceWrapper<T: Service<ServiceRequest>> {
    service: T,
}

impl<T> Service<ServiceRequest> for RouteServiceWrapper<T>
where
    T::Future: 'static,
    T: Service<ServiceRequest, Response = ServiceResponse, Error = Error>,
{
    type Response = ServiceResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        Box::pin(self.service.call(req))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use actix_rt::time::sleep;
    use bytes::Bytes;
    use serde_derive::Serialize;

    use crate::http::{Method, StatusCode};
    use crate::test::{call_service, init_service, read_body, TestRequest};
    use crate::{error, web, App, HttpResponse};

    #[derive(Serialize, PartialEq, Debug)]
    struct MyObject {
        name: String,
    }

    #[actix_rt::test]
    async fn test_route() {
        let mut srv = init_service(
            App::new()
                .service(
                    web::resource("/test")
                        .route(web::get().to(HttpResponse::Ok))
                        .route(web::put().to(|| async {
                            Err::<HttpResponse, _>(error::ErrorBadRequest("err"))
                        }))
                        .route(web::post().to(|| async {
                            sleep(Duration::from_millis(100)).await;
                            Ok::<_, ()>(HttpResponse::Created())
                        }))
                        .route(web::delete().to(|| async {
                            sleep(Duration::from_millis(100)).await;
                            Err::<HttpResponse, _>(error::ErrorBadRequest("err"))
                        })),
                )
                .service(web::resource("/json").route(web::get().to(|| async {
                    sleep(Duration::from_millis(25)).await;
                    web::Json(MyObject {
                        name: "test".to_string(),
                    })
                }))),
        )
        .await;

        let req = TestRequest::with_uri("/test")
            .method(Method::GET)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test")
            .method(Method::POST)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/test")
            .method(Method::PUT)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/test")
            .method(Method::DELETE)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/test")
            .method(Method::HEAD)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/json").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = read_body(resp).await;
        assert_eq!(body, Bytes::from_static(b"{\"name\":\"test\"}"));
    }
}
