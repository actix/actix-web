//! For middleware documentation, see [`ErrorHandlers`].

use std::{
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_service::{Service, Transform};
use ahash::AHashMap;
use futures_core::{future::LocalBoxFuture, ready};

use crate::{
    dev::{ServiceRequest, ServiceResponse},
    error::{Error, Result},
    http::StatusCode,
};

/// Return type for [`ErrorHandlers`] custom handlers.
pub enum ErrorHandlerResponse<B> {
    /// Immediate HTTP response.
    Response(ServiceResponse<B>),

    /// A future that resolves to an HTTP response.
    Future(LocalBoxFuture<'static, Result<ServiceResponse<B>, Error>>),
}

type ErrorHandler<B> = dyn Fn(ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>>;

/// Middleware for registering custom status code based error handlers.
///
/// Register handlers with the `ErrorHandlers::handler()` method to register a custom error handler
/// for a given status code. Handlers can modify existing responses or create completely new ones.
///
/// # Examples
/// ```
/// use actix_web::middleware::{ErrorHandlers, ErrorHandlerResponse};
/// use actix_web::{web, http, dev, App, HttpRequest, HttpResponse, Result};
///
/// fn render_500<B>(mut res: dev::ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
///     res.response_mut()
///        .headers_mut()
///        .insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("Error"));
///     Ok(ErrorHandlerResponse::Response(res))
/// }
///
/// let app = App::new()
///     .wrap(
///         ErrorHandlers::new()
///             .handler(http::StatusCode::INTERNAL_SERVER_ERROR, render_500),
///     )
///     .service(web::resource("/test")
///         .route(web::get().to(|| HttpResponse::Ok()))
///         .route(web::head().to(|| HttpResponse::MethodNotAllowed())
///     ));
/// ```
pub struct ErrorHandlers<B> {
    handlers: Handlers<B>,
}

type Handlers<B> = Rc<AHashMap<StatusCode, Box<ErrorHandler<B>>>>;

impl<B> Default for ErrorHandlers<B> {
    fn default() -> Self {
        ErrorHandlers {
            handlers: Rc::new(AHashMap::default()),
        }
    }
}

impl<B> ErrorHandlers<B> {
    /// Construct new `ErrorHandlers` instance.
    pub fn new() -> Self {
        ErrorHandlers::default()
    }

    /// Register error handler for specified status code.
    pub fn handler<F>(mut self, status: StatusCode, handler: F) -> Self
    where
        F: Fn(ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> + 'static,
    {
        Rc::get_mut(&mut self.handlers)
            .unwrap()
            .insert(status, Box::new(handler));
        self
    }
}

impl<S, B> Transform<S, ServiceRequest> for ErrorHandlers<B>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = ErrorHandlersMiddleware<S, B>;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let handlers = self.handlers.clone();
        Box::pin(async move { Ok(ErrorHandlersMiddleware { service, handlers }) })
    }
}

#[doc(hidden)]
pub struct ErrorHandlersMiddleware<S, B> {
    service: S,
    handlers: Handlers<B>,
}

impl<S, B> Service<ServiceRequest> for ErrorHandlersMiddleware<S, B>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = ErrorHandlersFuture<S::Future, B>;

    actix_service::forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let handlers = self.handlers.clone();
        let fut = self.service.call(req);
        ErrorHandlersFuture::ServiceFuture { fut, handlers }
    }
}

#[pin_project::pin_project(project = ErrorHandlersProj)]
pub enum ErrorHandlersFuture<Fut, B>
where
    Fut: Future,
{
    ServiceFuture {
        #[pin]
        fut: Fut,
        handlers: Handlers<B>,
    },
    HandlerFuture {
        fut: LocalBoxFuture<'static, Fut::Output>,
    },
}

impl<Fut, B> Future for ErrorHandlersFuture<Fut, B>
where
    Fut: Future<Output = Result<ServiceResponse<B>, Error>>,
{
    type Output = Fut::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project() {
            ErrorHandlersProj::ServiceFuture { fut, handlers } => {
                let res = ready!(fut.poll(cx))?;
                match handlers.get(&res.status()) {
                    Some(handler) => match handler(res)? {
                        ErrorHandlerResponse::Response(res) => Poll::Ready(Ok(res)),
                        ErrorHandlerResponse::Future(fut) => {
                            self.as_mut()
                                .set(ErrorHandlersFuture::HandlerFuture { fut });
                            self.poll(cx)
                        }
                    },
                    None => Poll::Ready(Ok(res)),
                }
            }
            ErrorHandlersProj::HandlerFuture { fut } => fut.as_mut().poll(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;
    use actix_utils::future::ok;
    use futures_util::future::FutureExt as _;

    use super::*;
    use crate::http::{header::CONTENT_TYPE, HeaderValue, StatusCode};
    use crate::test::{self, TestRequest};
    use crate::HttpResponse;

    #[allow(clippy::unnecessary_wraps)]
    fn render_500<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
        res.response_mut()
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));
        Ok(ErrorHandlerResponse::Response(res))
    }

    #[actix_rt::test]
    async fn test_handler() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::InternalServerError().finish()))
        };

        let mw = ErrorHandlers::new()
            .handler(StatusCode::INTERNAL_SERVER_ERROR, render_500)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

    #[allow(clippy::unnecessary_wraps)]
    fn render_500_async<B: 'static>(
        mut res: ServiceResponse<B>,
    ) -> Result<ErrorHandlerResponse<B>> {
        res.response_mut()
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));
        Ok(ErrorHandlerResponse::Future(ok(res).boxed_local()))
    }

    #[actix_rt::test]
    async fn test_handler_async() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::InternalServerError().finish()))
        };

        let mw = ErrorHandlers::new()
            .handler(StatusCode::INTERNAL_SERVER_ERROR, render_500_async)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp = test::call_service(&mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }
}
