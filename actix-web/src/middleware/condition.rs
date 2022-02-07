//! For middleware documentation, see [`Condition`].

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_service::{Service, Transform};
use futures_core::{future::LocalBoxFuture, ready};
use futures_util::future::FutureExt as _;
use pin_project_lite::pin_project;

use crate::{body::EitherBody, dev::ServiceResponse};

/// Middleware for conditionally enabling other middleware.
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

impl<S, T, Req, BE, BD, Err> Transform<S, Req> for Condition<T>
where
    S: Service<Req, Response = ServiceResponse<BD>, Error = Err> + 'static,
    T: Transform<S, Req, Response = ServiceResponse<BE>, Error = Err>,
    T::Future: 'static,
    T::InitError: 'static,
    T::Transform: 'static,
{
    type Response = ServiceResponse<EitherBody<BE, BD>>;
    type Error = Err;
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

impl<E, D, Req, BE, BD, Err> Service<Req> for ConditionMiddleware<E, D>
where
    E: Service<Req, Response = ServiceResponse<BE>, Error = Err>,
    D: Service<Req, Response = ServiceResponse<BD>, Error = Err>,
{
    type Response = ServiceResponse<EitherBody<BE, BD>>;
    type Error = Err;
    type Future = ConditionMiddlewareFuture<E::Future, D::Future>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self {
            ConditionMiddleware::Enable(service) => service.poll_ready(cx),
            ConditionMiddleware::Disable(service) => service.poll_ready(cx),
        }
    }

    fn call(&self, req: Req) -> Self::Future {
        match self {
            ConditionMiddleware::Enable(service) => ConditionMiddlewareFuture::Left {
                fut: service.call(req),
            },
            ConditionMiddleware::Disable(service) => ConditionMiddlewareFuture::Right {
                fut: service.call(req),
            },
        }
    }
}

pin_project! {
    #[project = EitherProj]
    pub enum ConditionMiddlewareFuture<L, R> {
        Left { #[pin] fut: L, },
        Right { #[pin] fut: R, },
    }
}

impl<L, R, BL, BR, Err> Future for ConditionMiddlewareFuture<L, R>
where
    L: Future<Output = Result<ServiceResponse<BL>, Err>>,
    R: Future<Output = Result<ServiceResponse<BR>, Err>>,
{
    type Output = Result<ServiceResponse<EitherBody<BL, BR>>, Err>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = match self.project() {
            EitherProj::Left { fut } => ready!(fut.poll(cx))?.map_into_left_body(),
            EitherProj::Right { fut } => ready!(fut.poll(cx))?.map_into_right_body(),
        };

        Poll::Ready(Ok(res))
    }
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;

    use super::*;
    use crate::{
        dev::{ServiceRequest, ServiceResponse},
        error::Result,
        http::{
            header::{HeaderValue, CONTENT_TYPE},
            StatusCode,
        },
        middleware::err_handlers::*,
        test::{self, TestRequest},
        HttpResponse,
    };

    fn assert_type<T>(_: &T) {}

    #[allow(clippy::unnecessary_wraps)]
    fn render_500<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
        res.response_mut()
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));

        Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
    }

    #[actix_rt::test]
    async fn test_handler_enabled() {
        let srv = |req: ServiceRequest| async move {
            let resp = HttpResponse::InternalServerError().message_body(String::new())?;
            Ok(req.into_response(resp))
        };

        let mw = ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, render_500);

        let mw = Condition::new(true, mw)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_type::<ServiceResponse<EitherBody<EitherBody<_, _>, String>>>(&resp);
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

    #[actix_rt::test]
    async fn test_handler_disabled() {
        let srv = |req: ServiceRequest| async move {
            let resp = HttpResponse::InternalServerError().message_body(String::new())?;
            Ok(req.into_response(resp))
        };

        let mw = ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, render_500);

        let mw = Condition::new(false, mw)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_type::<ServiceResponse<EitherBody<EitherBody<_, _>, String>>>(&resp);
        assert_eq!(resp.headers().get(CONTENT_TYPE), None);
    }
}
