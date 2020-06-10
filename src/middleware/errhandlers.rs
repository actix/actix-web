//! Custom handlers service for responses.
use std::rc::Rc;
use std::task::{Context, Poll};

use actix_service::{Service, Transform};
use futures_util::future::{ok, FutureExt, LocalBoxFuture, Ready};
use fxhash::FxHashMap;

use crate::dev::{ServiceRequest, ServiceResponse};
use crate::error::{Error, Result};
use crate::http::StatusCode;

/// Error handler response
pub enum ErrorHandlerResponse<B> {
    /// New http response got generated
    Response(ServiceResponse<B>),
    /// Result is a future that resolves to a new http response
    Future(LocalBoxFuture<'static, Result<ServiceResponse<B>, Error>>),
}

type ErrorHandler<B> = dyn Fn(ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>>;

/// `Middleware` for allowing custom handlers for responses.
///
/// You can use `ErrorHandlers::handler()` method  to register a custom error
/// handler for specific status code. You can modify existing response or
/// create completely new one.
///
/// ## Example
///
/// ```rust
/// use actix_web::middleware::errhandlers::{ErrorHandlers, ErrorHandlerResponse};
/// use actix_web::{web, http, dev, App, HttpRequest, HttpResponse, Result};
///
/// fn render_500<B>(mut res: dev::ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
///     res.response_mut()
///        .headers_mut()
///        .insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("Error"));
///     Ok(ErrorHandlerResponse::Response(res))
/// }
///
/// # fn main() {
/// let app = App::new()
///     .wrap(
///         ErrorHandlers::new()
///             .handler(http::StatusCode::INTERNAL_SERVER_ERROR, render_500),
///     )
///     .service(web::resource("/test")
///         .route(web::get().to(|| HttpResponse::Ok()))
///         .route(web::head().to(|| HttpResponse::MethodNotAllowed())
///     ));
/// # }
/// ```
pub struct ErrorHandlers<B> {
    handlers: Rc<FxHashMap<StatusCode, Box<ErrorHandler<B>>>>,
}

impl<B> Default for ErrorHandlers<B> {
    fn default() -> Self {
        ErrorHandlers {
            handlers: Rc::new(FxHashMap::default()),
        }
    }
}

impl<B> ErrorHandlers<B> {
    /// Construct new `ErrorHandlers` instance
    pub fn new() -> Self {
        ErrorHandlers::default()
    }

    /// Register error handler for specified status code
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

impl<S, B> Transform<S> for ErrorHandlers<B>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = ErrorHandlersMiddleware<S, B>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(ErrorHandlersMiddleware {
            service,
            handlers: self.handlers.clone(),
        })
    }
}

#[doc(hidden)]
pub struct ErrorHandlersMiddleware<S, B> {
    service: S,
    handlers: Rc<FxHashMap<StatusCode, Box<ErrorHandler<B>>>>,
}

impl<S, B> Service for ErrorHandlersMiddleware<S, B>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        let handlers = self.handlers.clone();
        let fut = self.service.call(req);

        async move {
            let res = fut.await?;

            if let Some(handler) = handlers.get(&res.status()) {
                match handler(res) {
                    Ok(ErrorHandlerResponse::Response(res)) => Ok(res),
                    Ok(ErrorHandlerResponse::Future(fut)) => fut.await,
                    Err(e) => Err(e),
                }
            } else {
                Ok(res)
            }
        }
        .boxed_local()
    }
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;
    use futures_util::future::ok;

    use super::*;
    use crate::http::{header::CONTENT_TYPE, HeaderValue, StatusCode};
    use crate::test::{self, TestRequest};
    use crate::HttpResponse;

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

        let mut mw = ErrorHandlers::new()
            .handler(StatusCode::INTERNAL_SERVER_ERROR, render_500)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp =
            test::call_service(&mut mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

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

        let mut mw = ErrorHandlers::new()
            .handler(StatusCode::INTERNAL_SERVER_ERROR, render_500_async)
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let resp =
            test::call_service(&mut mw, TestRequest::default().to_srv_request()).await;
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }
}
