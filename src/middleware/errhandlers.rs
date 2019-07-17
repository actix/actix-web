//! Custom handlers service for responses.
use std::rc::Rc;

use actix_service::{Service, Transform};
use futures::future::{err, ok, Either, Future, FutureResult};
use futures::Poll;
use hashbrown::HashMap;

use crate::dev::{ServiceRequest, ServiceResponse};
use crate::error::{Error, Result};
use crate::http::StatusCode;

/// Error handler response
pub enum ErrorHandlerResponse<B> {
    /// New http response got generated
    Response(ServiceResponse<B>),
    /// Result is a future that resolves to a new http response
    Future(Box<dyn Future<Item = ServiceResponse<B>, Error = Error>>),
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
/// fn main() {
///     let app = App::new()
///         .wrap(
///             ErrorHandlers::new()
///                 .handler(http::StatusCode::INTERNAL_SERVER_ERROR, render_500),
///         )
///         .service(web::resource("/test")
///             .route(web::get().to(|| HttpResponse::Ok()))
///             .route(web::head().to(|| HttpResponse::MethodNotAllowed())
///         ));
/// }
/// ```
pub struct ErrorHandlers<B> {
    handlers: Rc<HashMap<StatusCode, Box<ErrorHandler<B>>>>,
}

impl<B> Default for ErrorHandlers<B> {
    fn default() -> Self {
        ErrorHandlers {
            handlers: Rc::new(HashMap::new()),
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
    type Future = FutureResult<Self::Transform, Self::InitError>;

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
    handlers: Rc<HashMap<StatusCode, Box<ErrorHandler<B>>>>,
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
    type Future = Box<dyn Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        let handlers = self.handlers.clone();

        Box::new(self.service.call(req).and_then(move |res| {
            if let Some(handler) = handlers.get(&res.status()) {
                match handler(res) {
                    Ok(ErrorHandlerResponse::Response(res)) => Either::A(ok(res)),
                    Ok(ErrorHandlerResponse::Future(fut)) => Either::B(fut),
                    Err(e) => Either::A(err(e)),
                }
            } else {
                Either::A(ok(res))
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;
    use futures::future::ok;

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

    #[test]
    fn test_handler() {
        let srv = |req: ServiceRequest| {
            req.into_response(HttpResponse::InternalServerError().finish())
        };

        let mut mw = test::block_on(
            ErrorHandlers::new()
                .handler(StatusCode::INTERNAL_SERVER_ERROR, render_500)
                .new_transform(srv.into_service()),
        )
        .unwrap();

        let resp = test::call_service(&mut mw, TestRequest::default().to_srv_request());
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }

    fn render_500_async<B: 'static>(
        mut res: ServiceResponse<B>,
    ) -> Result<ErrorHandlerResponse<B>> {
        res.response_mut()
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));
        Ok(ErrorHandlerResponse::Future(Box::new(ok(res))))
    }

    #[test]
    fn test_handler_async() {
        let srv = |req: ServiceRequest| {
            req.into_response(HttpResponse::InternalServerError().finish())
        };

        let mut mw = test::block_on(
            ErrorHandlers::new()
                .handler(StatusCode::INTERNAL_SERVER_ERROR, render_500_async)
                .new_transform(srv.into_service()),
        )
        .unwrap();

        let resp = test::call_service(&mut mw, TestRequest::default().to_srv_request());
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }
}
