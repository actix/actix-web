use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::test::{call_service, init_service, TestRequest};
use actix_web::{HttpResponse, ResponseError};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use futures_util::lock::Mutex;
use actix_utils::future::{ok, Ready};

#[derive(Debug, Clone)]
pub struct MyError;

impl ResponseError for MyError {}

impl std::fmt::Display for MyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "A custom error")
    }
}

#[actix_web::get("/test")]
async fn test() -> Result<actix_web::HttpResponse, actix_web::error::Error> {
    Err(MyError)?;
    Ok(HttpResponse::NoContent().finish())
}

#[derive(Clone)]
pub struct SpyMiddleware(Arc<Mutex<Option<bool>>>);

impl<S, B> Transform<S, ServiceRequest> for SpyMiddleware
    where
        S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error>,
        S::Future: 'static,
        B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type Transform = Middleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(Middleware {
            was_error: self.0.clone(),
            service,
        })
    }
}

#[doc(hidden)]
pub struct Middleware<S> {
    was_error: Arc<Mutex<Option<bool>>>,
    service: S,
}

impl<S, B> Service<ServiceRequest> for Middleware<S>
    where
        S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error>,
        S::Future: 'static,
        B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let lock = self.was_error.clone();
        let response_future = self.service.call(req);
        Box::pin(async move {
            let response = response_future.await;
            if let Ok(success) = &response {
                *lock.lock().await = Some(success.response().error().is_some());
            }
            response
        })
    }
}

#[actix_rt::test]
async fn error_cause_should_be_propagated_to_middlewares() {
    let lock = Arc::new(Mutex::new(None));
    let spy_middleware = SpyMiddleware(lock.clone());

    let app = init_service(
        actix_web::App::new()
            .wrap(spy_middleware.clone())
            .service(test),
    )
        .await;

    call_service(&app, TestRequest::with_uri("/test").to_request()).await;

    let was_error_captured = lock.lock().await.unwrap();
    assert!(was_error_captured);
}
