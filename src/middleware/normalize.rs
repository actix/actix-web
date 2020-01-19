//! `Middleware` to normalize request's URI
use std::task::{Context, Poll};

use actix_http::http::{PathAndQuery, Uri};
use actix_service::{Service, Transform};
use bytes::Bytes;
use futures::future::{ok, Ready};
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
/// # fn main() {
/// let app = App::new()
///     .wrap(middleware::NormalizePath)
///     .service(
///         web::resource("/test")
///             .route(web::get().to(|| HttpResponse::Ok()))
///             .route(web::method(http::Method::HEAD).to(|| HttpResponse::MethodNotAllowed()))
///     );
/// # }
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
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(NormalizePathNormalization {
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

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, mut req: ServiceRequest) -> Self::Future {
        let head = req.head_mut();
        let path = head.uri.path();
        let original_len = path.len();
        let path = self.merge_slash.replace_all(path, "/");

        if original_len != path.len() {
            let mut parts = head.uri.clone().into_parts();
            let pq = parts.path_and_query.as_ref().unwrap();

            let path = if let Some(q) = pq.query() {
                Bytes::from(format!("{}?{}", path, q))
            } else {
                Bytes::copy_from_slice(path.as_bytes())
            };
            parts.path_and_query = Some(PathAndQuery::from_maybe_shared(path).unwrap());

            let uri = Uri::from_parts(parts).unwrap();
            req.match_info_mut().get_mut().update(&uri);
            req.head_mut().uri = uri;
        }

        self.service.call(req)
    }
}

#[cfg(test)]
mod tests {
    use actix_service::IntoService;

    use super::*;
    use crate::dev::ServiceRequest;
    use crate::test::{call_service, init_service, TestRequest};
    use crate::{web, App, HttpResponse};

    #[actix_rt::test]
    async fn test_wrap() {
        let mut app = init_service(
            App::new()
                .wrap(NormalizePath::default())
                .service(web::resource("/v1/something/").to(|| HttpResponse::Ok())),
        )
        .await;

        let req = TestRequest::with_uri("/v1//something////").to_request();
        let res = call_service(&mut app, req).await;
        assert!(res.status().is_success());
    }

    #[actix_rt::test]
    async fn test_in_place_normalization() {
        let srv = |req: ServiceRequest| {
            assert_eq!("/v1/something/", req.path());
            ok(req.into_response(HttpResponse::Ok().finish()))
        };

        let mut normalize = NormalizePath
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let req = TestRequest::with_uri("/v1//something////").to_srv_request();
        let res = normalize.call(req).await.unwrap();
        assert!(res.status().is_success());
    }

    #[actix_rt::test]
    async fn should_normalize_nothing() {
        const URI: &str = "/v1/something/";

        let srv = |req: ServiceRequest| {
            assert_eq!(URI, req.path());
            ok(req.into_response(HttpResponse::Ok().finish()))
        };

        let mut normalize = NormalizePath
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let req = TestRequest::with_uri(URI).to_srv_request();
        let res = normalize.call(req).await.unwrap();
        assert!(res.status().is_success());
    }
}
