//! `Middleware` for conditionally enables another middleware.
use actix_service::{Service, Transform};
use futures::future::{ok, Either, FutureResult, Map};
use futures::{Future, Poll};

/// `Middleware` for conditionally enables another middleware.
/// The controled middleware must not change the `Service` interfaces.
/// This means you cannot control such middlewares like `Logger` or `Compress`.
///
/// ## Usage
///
/// ```rust
/// use actix_web::middleware::{Condition, NormalizePath};
/// use actix_web::App;
///
/// fn main() {
///     let enable_normalize = std::env::var("NORMALIZE_PATH") == Ok("true".into());
///     let app = App::new()
///         .wrap(Condition::new(enable_normalize, NormalizePath));
/// }
/// ```
pub struct Condition<T> {
    trans: T,
    enable: bool,
}

impl<T> Condition<T> {
    pub fn new(enable: bool, trans: T) -> Self {
        Self { trans, enable }
    }
}

impl<S, T> Transform<S> for Condition<T>
where
    S: Service,
    T: Transform<S, Request = S::Request, Response = S::Response, Error = S::Error>,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type InitError = T::InitError;
    type Transform = ConditionMiddleware<T::Transform, S>;
    type Future = Either<
        Map<T::Future, fn(T::Transform) -> Self::Transform>,
        FutureResult<Self::Transform, Self::InitError>,
    >;

    fn new_transform(&self, service: S) -> Self::Future {
        if self.enable {
            let f = self
                .trans
                .new_transform(service)
                .map(ConditionMiddleware::Enable as fn(T::Transform) -> Self::Transform);
            Either::A(f)
        } else {
            Either::B(ok(ConditionMiddleware::Disable(service)))
        }
    }
}

pub enum ConditionMiddleware<E, D> {
    Enable(E),
    Disable(D),
}

impl<E, D> Service for ConditionMiddleware<E, D>
where
    E: Service,
    D: Service<Request = E::Request, Response = E::Response, Error = E::Error>,
{
    type Request = E::Request;
    type Response = E::Response;
    type Error = E::Error;
    type Future = Either<E::Future, D::Future>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        use ConditionMiddleware::*;
        match self {
            Enable(service) => service.poll_ready(),
            Disable(service) => service.poll_ready(),
        }
    }

    fn call(&mut self, req: E::Request) -> Self::Future {
        use ConditionMiddleware::*;
        match self {
            Enable(service) => Either::A(service.call(req)),
            Disable(service) => Either::B(service.call(req)),
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;

    use super::*;
    use crate::dev::{ServiceRequest, ServiceResponse};
    use crate::error::Result;
    use crate::http::{header::CONTENT_TYPE, HeaderValue, StatusCode};
    use crate::middleware::errhandlers::*;
    use crate::test::{self, TestRequest};
    use crate::HttpResponse;

    fn render_500<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
        res.response_mut()
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("0001"));
        Ok(ErrorHandlerResponse::Response(res))
    }

    #[test]
    fn test_handler_enabled() {
        let srv = |req: ServiceRequest| {
            req.into_response(HttpResponse::InternalServerError().finish())
        };

        let mw =
            ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, render_500);

        let mut mw =
            test::block_on(Condition::new(true, mw).new_transform(srv.into_service()))
                .unwrap();
        let resp = test::call_service(&mut mw, TestRequest::default().to_srv_request());
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "0001");
    }
    #[test]
    fn test_handler_disabled() {
        let srv = |req: ServiceRequest| {
            req.into_response(HttpResponse::InternalServerError().finish())
        };

        let mw =
            ErrorHandlers::new().handler(StatusCode::INTERNAL_SERVER_ERROR, render_500);

        let mut mw =
            test::block_on(Condition::new(false, mw).new_transform(srv.into_service()))
                .unwrap();

        let resp = test::call_service(&mut mw, TestRequest::default().to_srv_request());
        assert_eq!(resp.headers().get(CONTENT_TYPE), None);
    }
}
