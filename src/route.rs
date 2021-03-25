#![allow(clippy::rc_buffer)] // inner value is mutated before being shared (`Rc::get_mut`)

use std::{future::Future, rc::Rc};

use actix_http::{http::Method, Error};
use actix_service::{
    boxed::{self, BoxService, BoxServiceFactory},
    Service, ServiceFactory,
};
use futures_core::future::LocalBoxFuture;

use crate::extract::FromRequest;
use crate::guard::{self, Guard};
use crate::handler::{Handler, HandlerService};
use crate::responder::Responder;
use crate::service::{ServiceRequest, ServiceResponse};
use crate::HttpResponse;

/// Resource route definition
///
/// Route uses builder-like pattern for configuration.
/// If handler is not explicitly set, default *404 Not Found* handler is used.
pub struct Route {
    service: BoxServiceFactory<(), ServiceRequest, ServiceResponse, Error, ()>,
    guards: Rc<Vec<Box<dyn Guard>>>,
}

impl Route {
    /// Create new route which matches any request.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Route {
        Route {
            service: boxed::factory(HandlerService::new(HttpResponse::NotFound)),
            guards: Rc::new(Vec::new()),
        }
    }

    pub(crate) fn take_guards(&mut self) -> Vec<Box<dyn Guard>> {
        std::mem::take(Rc::get_mut(&mut self.guards).unwrap())
    }
}

impl ServiceFactory<ServiceRequest> for Route {
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = RouteService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let fut = self.service.new_service(());
        let guards = self.guards.clone();

        Box::pin(async move {
            let service = fut.await?;
            Ok(RouteService { service, guards })
        })
    }
}

pub struct RouteService {
    service: BoxService<ServiceRequest, ServiceResponse, Error>,
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

    actix_service::forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        self.service.call(req)
    }
}

impl Route {
    /// Add method guard to the route.
    ///
    /// ```
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
    /// ```
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
    /// ```
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
    /// ```
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
        self.service = boxed::factory(HandlerService::new(handler));
        self
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
        let srv = init_service(
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
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test")
            .method(Method::POST)
            .to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/test")
            .method(Method::PUT)
            .to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/test")
            .method(Method::DELETE)
            .to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/test")
            .method(Method::HEAD)
            .to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/json").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = read_body(resp).await;
        assert_eq!(body, Bytes::from_static(b"{\"name\":\"test\"}"));
    }
}
