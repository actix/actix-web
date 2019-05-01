//! `Middleware` to normalize request's URI

use actix_service::{Service, Transform};
use futures::future::{self, FutureResult};
use regex::Regex;

use crate::service::{ServiceRequest, ServiceResponse};
use crate::Error;

#[derive(Default, Clone, Copy)]
/// `Middleware` to normalize request's URI in place
///
/// Performs following:
///
/// - Merges multiple slashes into one.
///
/// ```rust
/// use actix_web::{web, http, middleware, App, HttpResponse};
///
/// fn main() {
///     let app = App::new()
///         .wrap(middleware::NormalizePath)
///         .service(
///             web::resource("/test")
///                 .route(web::get().to(|| HttpResponse::Ok()))
///                 .route(web::method(http::Method::HEAD).to(|| HttpResponse::MethodNotAllowed()))
///         );
/// }
/// ```

pub struct NormalizePath;

impl<S, B> Transform<S> for NormalizePath
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = NormalizePathNormalization<S>;
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        future::ok(NormalizePathNormalization {
            service,
            merge_slash: Regex::new("//+").unwrap(),
        })
    }
}

pub struct NormalizePathNormalization<S> {
    service: S,
    merge_slash: Regex,
}

impl<S, B> Service for NormalizePathNormalization<S>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = S::Future;

    fn poll_ready(&mut self) -> futures::Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, mut req: ServiceRequest) -> Self::Future {
        let head = req.head_mut();

        let path = head.uri.path();
        let original_len = path.len();
        let path = self.merge_slash.replace_all(path, "/");

        if original_len != path.len() {
            head.uri = path.parse().unwrap();
        }

        self.service.call(req)
    }
}

#[cfg(test)]
mod tests {
    use actix_service::FnService;

    use super::*;
    use crate::dev::ServiceRequest;
    use crate::test::{block_on, TestRequest};
    use crate::HttpResponse;

    #[test]
    fn test_in_place_normalization() {
        let srv = FnService::new(|req: ServiceRequest| {
            assert_eq!("/v1/something/", req.path());
            req.into_response(HttpResponse::Ok().finish())
        });

        let mut normalize = block_on(NormalizePath.new_transform(srv)).unwrap();

        let req = TestRequest::with_uri("/v1//something////").to_srv_request();
        let res = block_on(normalize.call(req)).unwrap();
        assert!(res.status().is_success());
    }

    #[test]
    fn should_normalize_nothing() {
        const URI: &str = "/v1/something/";

        let srv = FnService::new(|req: ServiceRequest| {
            assert_eq!(URI, req.path());
            req.into_response(HttpResponse::Ok().finish())
        });

        let mut normalize = block_on(NormalizePath.new_transform(srv)).unwrap();

        let req = TestRequest::with_uri(URI).to_srv_request();
        let res = block_on(normalize.call(req)).unwrap();
        assert!(res.status().is_success());
    }

}
