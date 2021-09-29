//! For middleware documentation, see [`Condition`].

use std::task::{Context, Poll};

use actix_service::{Service, Transform};
use actix_utils::future::Either;
use std::future::{ready, Future, Ready};
use std::ops::DerefMut;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Mutex;

/// Middleware for conditionally enabling other middleware.
///
/// The controlled middleware must not change the `Service` interfaces. This means you cannot
/// control such middlewares like `Logger` or `Compress` directly. See the [`Compat`](super::Compat)
/// middleware for a workaround.
///
/// # Examples
/// ```
/// use actix_web::middleware::{Condition, NormalizePath, TrailingSlash, conditionally, optionally, optionally_fut};
/// use actix_web::App;
/// use std::future::ready;
///
/// let enable_normalize = std::env::var("NORMALIZE_PATH").is_ok();
/// let config_opt = Some(TrailingSlash::Trim);
/// let config_opt_future = ready(Some(TrailingSlash::Always));
/// let future = ready(Some(NormalizePath::new(TrailingSlash::MergeOnly)));
///
/// let app = App::new()
///     .wrap(conditionally(enable_normalize, NormalizePath::default()))
///     .wrap(optionally(config_opt, |mode| NormalizePath::new(mode)))
///     .wrap(optionally_fut(config_opt_future, |mode| NormalizePath::new(mode)))
///     .wrap(futurally(future));
/// ```

pub struct Condition<T, F>(Rc<Mutex<F>>)
where
    F: Future<Output = Option<T>> + Unpin + 'static;

pub fn futurally<T, F>(transformer: F) -> Condition<T, F>
where
    F: Future<Output = Option<T>> + Unpin + 'static,
{
    Condition(Rc::new(Mutex::new(transformer)))
}

pub fn conditionally<T>(enable: bool, transformer: T) -> Condition<T, Ready<Option<T>>> {
    if enable {
        Condition::<T, Ready<Option<T>>>(Rc::new(Mutex::new(ready(Some(transformer)))))
    } else {
        Condition::<T, Ready<Option<T>>>(Rc::new(Mutex::new(ready(None))))
    }
}

pub fn optionally<T, A, FACTORY>(
    condition: Option<A>,
    transformer: FACTORY,
) -> Condition<T, impl Future<Output = Option<T>>>
where
    FACTORY: FnOnce(A) -> T,
{
    match condition {
        Some(v) => {
            Condition::<T, Ready<Option<T>>>(Rc::new(Mutex::new(ready(Some(transformer(v))))))
        }
        None => Condition::<T, Ready<Option<T>>>(Rc::new(Mutex::new(ready(None)))),
    }
}

pub fn optionally_fut<T, A, F2, FACTORY>(
    condition: F2,
    transformer: FACTORY,
) -> Condition<T, impl Future<Output = Option<T>> + Unpin + 'static>
where
    F2: Future<Output = Option<A>> + Unpin + 'static,
    FACTORY: FnOnce(A) -> T,
{
    Condition(Rc::new(Mutex::new(Box::pin(async move {
        match condition.await {
            Some(v) => Some(transformer(v)),
            None => None,
        }
    }))))
}

impl<S, T, Req, F> Transform<S, Req> for Condition<T, F>
where
    S: Service<Req> + 'static,
    T: Transform<S, Req, Response = S::Response, Error = S::Error>,
    T::Future: 'static,
    T::InitError: 'static,
    T::Transform: 'static,
    F: Future<Output = Option<T>> + Unpin + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Transform = ConditionMiddleware<T::Transform, S>;
    type InitError = T::InitError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Transform, Self::InitError>>>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let mutex = self.0.clone();

        Box::pin(async move {
            let mut lock = mutex.lock().unwrap();

            let transformer = lock.deref_mut().await;

            match transformer {
                Some(transformer) => {
                    let fut = transformer.new_transform(service);
                    let wrapped_svc = fut.await?;
                    Ok(ConditionMiddleware::Enable(wrapped_svc))
                }
                None => Ok(ConditionMiddleware::Disable(service)),
            }
        })
    }
}

pub enum ConditionMiddleware<E, D> {
    Enable(E),
    Disable(D),
}

impl<E, D, Req> Service<Req> for ConditionMiddleware<E, D>
where
    E: Service<Req>,
    D: Service<Req, Response = E::Response, Error = E::Error>,
{
    type Response = E::Response;
    type Error = E::Error;
    type Future = Either<E::Future, D::Future>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self {
            ConditionMiddleware::Enable(service) => service.poll_ready(cx),
            ConditionMiddleware::Disable(service) => service.poll_ready(cx),
        }
    }

    fn call(&self, req: Req) -> Self::Future {
        match self {
            ConditionMiddleware::Enable(service) => Either::left(service.call(req)),
            ConditionMiddleware::Disable(service) => Either::right(service.call(req)),
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;
    use actix_utils::future::ok;

    use super::*;
    use crate::{
        dev::{ServiceRequest, ServiceResponse},
        error::Result,
        http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
        middleware::err_handlers::*,
        test::{self, TestRequest},
        HttpResponse,
    };

    #[allow(clippy::unnecessary_wraps)]
    fn render_500<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
        res.response_mut()
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));
        Ok(ErrorHandlerResponse::Response(res))
    }

    #[actix_rt::test]
    async fn test_handler_enabled() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::InternalServerError().finish()))
        };

        let mw = ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, render_500);

        let mw = conditionally(true, mw)
            .new_transform(srv.into_service())
            .await
            .unwrap();
        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

    #[actix_rt::test]
    async fn test_handler_disabled() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::InternalServerError().finish()))
        };

        let mw = ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, render_500);

        let mw = conditionally(false, mw)
            .new_transform(srv.into_service())
            .await
            .unwrap();
        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE), None);
    }

    #[actix_rt::test]
    async fn test_handler_optional_some() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::InternalServerError().finish()))
        };

        let mw = optionally(Some(StatusCode::INTERNAL_SERVER_ERROR), |status| {
            ErrorHandlers::new().handler(status, render_500)
        })
        .new_transform(srv.into_service())
        .await
        .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

    #[actix_rt::test]
    async fn test_handler_optional_none() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::InternalServerError().finish()))
        };

        let mw = optionally(None, |status| {
            ErrorHandlers::new().handler(status, render_500)
        })
        .new_transform(srv.into_service())
        .await
        .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE), None);
    }

    #[actix_rt::test]
    async fn test_handler_optional_future_some() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::InternalServerError().finish()))
        };

        let mw = optionally_fut(ready(Some(StatusCode::INTERNAL_SERVER_ERROR)), |status| {
            ErrorHandlers::new().handler(status, render_500)
        })
        .new_transform(srv.into_service())
        .await
        .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

    #[actix_rt::test]
    async fn test_handler_optional_future_none() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::InternalServerError().finish()))
        };

        let mw = optionally_fut(ready(None), |status| {
            ErrorHandlers::new().handler(status, render_500)
        })
        .new_transform(srv.into_service())
        .await
        .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE), None);
    }

    #[actix_rt::test]
    async fn test_handler_futurally_enabled() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::InternalServerError().finish()))
        };

        let mw = futurally(ready(Some(
            ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, render_500),
        )))
        .new_transform(srv.into_service())
        .await
        .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

    #[actix_rt::test]
    async fn test_handler_futurally_disabled() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::InternalServerError().finish()))
        };

        let none: Option<ErrorHandlers<_>> = None;

        let mw = futurally(ready(none))
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE), None);
    }
}
