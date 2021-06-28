//! For middleware documentation, see [`Condition`].

use std::task::{Context, Poll};

use actix_service::{Service, Transform};
use actix_utils::future::Either;
use futures_core::future::LocalBoxFuture;
use futures_util::future::FutureExt as _;

/// Middleware for conditionally enabling other middleware.
///
/// The controlled middleware must not change the `Service` interfaces. This means you cannot
/// control such middlewares like `Logger` or `Compress` directly. See the [`Compat`](super::Compat)
/// middleware for a workaround.
///
/// # Examples
/// ```
/// use actix_web::middleware::{Condition, NormalizePath};
/// use actix_web::App;
///
/// let enable_normalize = std::env::var("NORMALIZE_PATH").is_ok();
/// let app = App::new()
///     .wrap(Condition::new(enable_normalize, NormalizePath::default()));
/// ```
pub struct Condition<T> {
    transformer: T,
    enable: bool,
}

impl<T> Condition<T> {
    pub fn new(enable: bool, transformer: T) -> Self {
        Self {
            transformer,
            enable,
        }
    }
}

impl<S, T, Req> Transform<S, Req> for Condition<T>
where
    S: Service<Req> + 'static,
    T: Transform<S, Req, Response = S::Response, Error = S::Error>,
    T::Future: 'static,
    T::InitError: 'static,
    T::Transform: 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Transform = ConditionMiddleware<T::Transform, S>;
    type InitError = T::InitError;
    type Future = LocalBoxFuture<'static, Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        if self.enable {
            let fut = self.transformer.new_transform(service);
            async move {
                let wrapped_svc = fut.await?;
                Ok(ConditionMiddleware::Enable(wrapped_svc))
            }
            .boxed_local()
        } else {
            async move { Ok(ConditionMiddleware::Disable(service)) }.boxed_local()
        }
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

        let mw = Condition::new(true, mw)
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

        let mw = Condition::new(false, mw)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE), None);
    }
}
