use std::collections::HashMap;

use error::Result;
use http::StatusCode;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::{Middleware, Response};

type ErrorHandler<S> = Fn(&mut HttpRequest<S>, HttpResponse) -> Result<Response>;

/// `Middleware` for allowing custom handlers for responses.
///
/// You can use `ErrorHandlers::handler()` method  to register a custom error
/// handler for specific status code. You can modify existing response or
/// create completely new one.
///
/// ## Example
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{http, App, HttpRequest, HttpResponse, Result};
/// use actix_web::middleware::{Response, ErrorHandlers};
///
/// fn render_500<S>(_: &mut HttpRequest<S>, resp: HttpResponse) -> Result<Response> {
///    let mut builder = resp.into_builder();
///    builder.header(http::header::CONTENT_TYPE, "application/json");
///    Ok(Response::Done(builder.into()))
/// }
///
/// fn main() {
///     let app = App::new()
///         .middleware(
///             ErrorHandlers::new()
///                 .handler(http::StatusCode::INTERNAL_SERVER_ERROR, render_500))
///         .resource("/test", |r| {
///              r.method(http::Method::GET).f(|_| HttpResponse::Ok());
///              r.method(http::Method::HEAD).f(|_| HttpResponse::MethodNotAllowed());
///         })
///         .finish();
/// }
/// ```
pub struct ErrorHandlers<S> {
    handlers: HashMap<StatusCode, Box<ErrorHandler<S>>>,
}

impl<S> Default for ErrorHandlers<S> {
    fn default() -> Self {
        ErrorHandlers {
            handlers: HashMap::new(),
        }
    }
}

impl<S> ErrorHandlers<S> {
    /// Construct new `ErrorHandlers` instance
    pub fn new() -> Self {
        ErrorHandlers::default()
    }

    /// Register error handler for specified status code
    pub fn handler<F>(mut self, status: StatusCode, handler: F) -> Self
    where
        F: Fn(&mut HttpRequest<S>, HttpResponse) -> Result<Response> + 'static,
    {
        self.handlers.insert(status, Box::new(handler));
        self
    }
}

impl<S: 'static> Middleware<S> for ErrorHandlers<S> {
    fn response(
        &self, req: &mut HttpRequest<S>, resp: HttpResponse,
    ) -> Result<Response> {
        if let Some(handler) = self.handlers.get(&resp.status()) {
            handler(req, resp)
        } else {
            Ok(Response::Done(resp))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::CONTENT_TYPE;
    use http::StatusCode;

    fn render_500<S>(_: &mut HttpRequest<S>, resp: HttpResponse) -> Result<Response> {
        let mut builder = resp.into_builder();
        builder.header(CONTENT_TYPE, "0001");
        Ok(Response::Done(builder.into()))
    }

    #[test]
    fn test_handler() {
        let mw =
            ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, render_500);

        let mut req = HttpRequest::default();
        let resp = HttpResponse::InternalServerError().finish();
        let resp = match mw.response(&mut req, resp) {
            Ok(Response::Done(resp)) => resp,
            _ => panic!(),
        };
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");

        let resp = HttpResponse::Ok().finish();
        let resp = match mw.response(&mut req, resp) {
            Ok(Response::Done(resp)) => resp,
            _ => panic!(),
        };
        assert!(!resp.headers().contains_key(CONTENT_TYPE));
    }
}
