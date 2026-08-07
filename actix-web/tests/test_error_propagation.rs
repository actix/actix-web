use std::sync::Arc;

use actix_utils::future::{ok, Ready};
use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    get,
    http::{header, StatusCode},
    middleware::{from_fn, Next},
    test::{call_service, init_service, TestRequest},
    web, App, Error, HttpResponse, ResponseError,
};
use futures_core::future::LocalBoxFuture;
use futures_util::lock::Mutex;

#[derive(Debug, Clone)]
pub struct MyError;

impl ResponseError for MyError {}

impl std::fmt::Display for MyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "A custom error")
    }
}

#[get("/test")]
async fn test() -> Result<actix_web::HttpResponse, actix_web::error::Error> {
    Err(MyError.into())
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
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

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

async fn inner_error_middleware<B>(
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse, Error> {
    next.call(req).await?;
    Err(actix_web::error::ErrorBadRequest("inner middleware error"))
}

async fn cors_like_middleware<B>(
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse<B>, Error> {
    let origin = req.headers().get(header::ORIGIN).cloned();

    match next.call(req).await {
        Ok(mut res) => {
            if let Some(origin) = origin {
                res.headers_mut()
                    .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
            }

            Ok(res)
        }
        Err(mut err) => {
            if let Some(origin) = origin {
                err.add_response_mapper(move |mut res| {
                    res.headers_mut()
                        .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
                    res
                });
            }

            Err(err)
        }
    }
}

#[actix_rt::test]
async fn error_response_mapper_allows_cors_like_middleware_to_augment_errors() {
    let srv = actix_test::start(|| {
        App::new()
            // Emulate an inner middleware that propagates an error rather than constructing a
            // response. This currently prevents response middleware such as actix-cors from
            // augmenting the eventual error response.
            .wrap(from_fn(inner_error_middleware))
            // Emulate actix-cors as the outer middleware. Successful responses are augmented
            // directly; errors carry a mapper that applies the same header during conversion.
            .wrap(from_fn(cors_like_middleware))
            .route("/", web::get().to(HttpResponse::Ok))
    });

    let res = srv
        .get("/")
        .insert_header((header::ORIGIN, "https://example.com"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        res.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        Some(&header::HeaderValue::from_static("https://example.com")),
    );

    srv.stop().await;
}
