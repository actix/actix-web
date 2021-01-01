//! `Middleware` for enabling any middleware to be used in `Resource`, `Scope` and `Condition`.
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_http::body::Body;
use actix_http::body::{MessageBody, ResponseBody};
use actix_service::{Service, Transform};
use futures_core::future::LocalBoxFuture;
use futures_core::ready;

use crate::error::Error;
use crate::service::ServiceResponse;

/// `Middleware` for enabling any middleware to be used in `Resource`, `Scope` and `Condition`.
///
///
/// ## Usage
///
/// ```rust
/// use actix_web::middleware::{Logger, Scoped};
/// use actix_web::{App, web};
///
/// # fn main() {
/// let logger = Logger::default();
///
/// // this would not compile
/// // let app = App::new().service(web::scope("scoped").wrap(logger));
///
/// // by using scoped middleware we can use logger in scope.
/// let app = App::new().service(web::scope("scoped").wrap(Scoped::new(logger)));
/// # }
/// ```
pub struct Scoped<T> {
    transform: T,
}

impl<T> Scoped<T> {
    pub fn new(transform: T) -> Self {
        Self { transform }
    }
}

impl<S, T> Transform<S> for Scoped<T>
where
    S: Service,
    T: Transform<S>,
    T::Future: 'static,
    T::Response: MapServiceResponseBody,
    Error: From<T::Error>,
{
    type Request = T::Request;
    type Response = ServiceResponse;
    type Error = Error;
    type Transform = ScopedMiddleware<T::Transform>;
    type InitError = T::InitError;
    type Future = LocalBoxFuture<'static, Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let fut = self.transform.new_transform(service);
        Box::pin(async move {
            let service = fut.await?;
            Ok(ScopedMiddleware { service })
        })
    }
}

pub struct ScopedMiddleware<S> {
    service: S,
}

impl<S> Service for ScopedMiddleware<S>
where
    S: Service,
    S::Response: MapServiceResponseBody,
    Error: From<S::Error>,
{
    type Request = S::Request;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = ScopedFuture<S>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx).map_err(From::from)
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let fut = self.service.call(req);
        ScopedFuture { fut }
    }
}

#[doc(hidden)]
#[pin_project::pin_project]
pub struct ScopedFuture<S>
where
    S: Service,
{
    #[pin]
    fut: S::Future,
}

impl<S> Future for ScopedFuture<S>
where
    S: Service,
    S::Response: MapServiceResponseBody,
    Error: From<S::Error>,
{
    type Output = Result<ServiceResponse, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = ready!(self.project().fut.poll(cx))?;
        Poll::Ready(Ok(res.map_body()))
    }
}

// private trait for convert ServiceResponse's ResponseBody<B> type
// to ResponseBody::Other(Body::Message)
#[doc(hidden)]
pub trait MapServiceResponseBody {
    fn map_body(self) -> ServiceResponse;
}

impl<B: MessageBody + Unpin + 'static> MapServiceResponseBody for ServiceResponse<B> {
    fn map_body(self) -> ServiceResponse {
        self.map_body(|_, body| ResponseBody::Other(Body::from_message(body)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use actix_service::IntoService;

    use crate::dev::ServiceRequest;
    use crate::http::StatusCode;
    use crate::middleware::{Compress, Condition, Logger};
    use crate::test::{call_service, init_service, TestRequest};
    use crate::App;
    use crate::{web, HttpResponse};

    #[actix_rt::test]
    async fn test_scope_middleware() {
        let logger = Logger::default();
        let compress = Compress::default();

        let mut srv = init_service(
            App::new().service(
                web::scope("app")
                    .wrap(Scoped::new(logger))
                    .wrap(Scoped::new(compress))
                    .service(
                        web::resource("/test").route(web::get().to(HttpResponse::Ok)),
                    ),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/test").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_resource_scope_middleware() {
        let logger = Logger::default();
        let compress = Compress::default();

        let mut srv = init_service(
            App::new().service(
                web::resource("app/test")
                    .wrap(Scoped::new(logger))
                    .wrap(Scoped::new(compress))
                    .route(web::get().to(HttpResponse::Ok)),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/test").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_condition_scope_middleware() {
        let srv = |req: ServiceRequest| {
            Box::pin(async move {
                Ok(req.into_response(HttpResponse::InternalServerError().finish()))
            })
        };

        let logger = Logger::default();

        let mut mw = Condition::new(true, Scoped::new(logger))
            .new_transform(srv.into_service())
            .await
            .unwrap();
        let resp = call_service(&mut mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
