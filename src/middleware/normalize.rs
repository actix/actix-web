//! `Middleware` to normalize request's URI
use std::task::{Context, Poll};

use actix_http::http::{PathAndQuery, Uri};
use actix_service::{Service, Transform};
use bytes::Bytes;
use futures_util::future::{ok, Ready};
use regex::Regex;

use crate::service::{ServiceRequest, ServiceResponse};
use crate::Error;

/// To be used when constructing `NormalizePath` to define it's behavior.
#[non_exhaustive]
#[derive(Clone, Copy)]
pub enum TrailingSlash {
    /// Always add a trailing slash to the end of the path.
    /// This will require all routes to end in a trailing slash for them to be accessible.
    Always,
    /// Only merge any present multiple trailing slashes.
    ///
    /// Note: This option provides the best compatibility with the v2 version of this middlware.
    MergeOnly,
    /// Trim trailing slashes from the end of the path.
    Trim,
}

impl Default for TrailingSlash {
    fn default() -> Self {
        TrailingSlash::Always
    }
}

#[derive(Default, Clone, Copy)]
/// `Middleware` to normalize request's URI in place
///
/// Performs following:
///
/// - Merges multiple slashes into one.
/// - Appends a trailing slash if one is not present, removes one if present, or keeps trailing
///   slashes as-is, depending on the supplied `TrailingSlash` variant.
///
/// ```rust
/// use actix_web::{web, http, middleware, App, HttpResponse};
///
/// # fn main() {
/// let app = App::new()
///     .wrap(middleware::NormalizePath::default())
///     .service(
///         web::resource("/test")
///             .route(web::get().to(|| HttpResponse::Ok()))
///             .route(web::method(http::Method::HEAD).to(|| HttpResponse::MethodNotAllowed()))
///     );
/// # }
/// ```

pub struct NormalizePath(TrailingSlash);

impl NormalizePath {
    /// Create new `NormalizePath` middleware with the specified trailing slash style.
    pub fn new(trailing_slash_style: TrailingSlash) -> Self {
        NormalizePath(trailing_slash_style)
    }
}

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
            trailing_slash_behavior: self.0,
        })
    }
}

#[doc(hidden)]
pub struct NormalizePathNormalization<S> {
    service: S,
    merge_slash: Regex,
    trailing_slash_behavior: TrailingSlash,
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

        let original_path = head.uri.path();

        // Either adds a string to the end (duplicates will be removed anyways) or trims all slashes from the end
        let path = match self.trailing_slash_behavior {
            TrailingSlash::Always => original_path.to_string() + "/",
            TrailingSlash::MergeOnly => original_path.to_string(),
            TrailingSlash::Trim => original_path.trim_end_matches('/').to_string(),
        };

        // normalize multiple /'s to one /
        let path = self.merge_slash.replace_all(&path, "/");

        // Ensure root paths are still resolvable. If resulting path is blank after previous step
        // it means the path was one or more slashes. Reduce to single slash.
        let path = if path.is_empty() { "/" } else { path.as_ref() };

        // Check whether the path has been changed
        //
        // This check was previously implemented as string length comparison
        //
        // That approach fails when a trailing slash is added,
        // and a duplicate slash is removed,
        // since the length of the strings remains the same
        //
        // For example, the path "/v1//s" will be normalized to "/v1/s/"
        // Both of the paths have the same length,
        // so the change can not be deduced from the length comparison
        if path != original_path {
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
                .service(web::resource("/").to(HttpResponse::Ok))
                .service(web::resource("/v1/something/").to(HttpResponse::Ok)),
        )
        .await;

        let req = TestRequest::with_uri("/").to_request();
        let res = call_service(&mut app, req).await;
        assert!(res.status().is_success());

        let req = TestRequest::with_uri("/?query=test").to_request();
        let res = call_service(&mut app, req).await;
        assert!(res.status().is_success());

        let req = TestRequest::with_uri("///").to_request();
        let res = call_service(&mut app, req).await;
        assert!(res.status().is_success());

        let req = TestRequest::with_uri("/v1//something////").to_request();
        let res = call_service(&mut app, req).await;
        assert!(res.status().is_success());

        let req2 = TestRequest::with_uri("//v1/something").to_request();
        let res2 = call_service(&mut app, req2).await;
        assert!(res2.status().is_success());

        let req3 = TestRequest::with_uri("//v1//////something").to_request();
        let res3 = call_service(&mut app, req3).await;
        assert!(res3.status().is_success());

        let req4 = TestRequest::with_uri("/v1//something").to_request();
        let res4 = call_service(&mut app, req4).await;
        assert!(res4.status().is_success());
    }

    #[actix_rt::test]
    async fn trim_trailing_slashes() {
        let mut app = init_service(
            App::new()
                .wrap(NormalizePath(TrailingSlash::Trim))
                .service(web::resource("/").to(HttpResponse::Ok))
                .service(web::resource("/v1/something").to(HttpResponse::Ok)),
        )
        .await;

        // root paths should still work
        let req = TestRequest::with_uri("/").to_request();
        let res = call_service(&mut app, req).await;
        assert!(res.status().is_success());

        let req = TestRequest::with_uri("/?query=test").to_request();
        let res = call_service(&mut app, req).await;
        assert!(res.status().is_success());

        let req = TestRequest::with_uri("///").to_request();
        let res = call_service(&mut app, req).await;
        assert!(res.status().is_success());

        let req = TestRequest::with_uri("/v1/something////").to_request();
        let res = call_service(&mut app, req).await;
        assert!(res.status().is_success());

        let req2 = TestRequest::with_uri("/v1/something/").to_request();
        let res2 = call_service(&mut app, req2).await;
        assert!(res2.status().is_success());

        let req3 = TestRequest::with_uri("//v1//something//").to_request();
        let res3 = call_service(&mut app, req3).await;
        assert!(res3.status().is_success());

        let req4 = TestRequest::with_uri("//v1//something").to_request();
        let res4 = call_service(&mut app, req4).await;
        assert!(res4.status().is_success());
    }

    #[actix_rt::test]
    async fn keep_trailing_slash_unchange() {
        let mut app = init_service(
            App::new()
                .wrap(NormalizePath(TrailingSlash::MergeOnly))
                .service(web::resource("/").to(HttpResponse::Ok))
                .service(web::resource("/v1/something").to(HttpResponse::Ok))
                .service(web::resource("/v1/").to(HttpResponse::Ok)),
        )
        .await;

        let tests = vec![
            ("/", true), // root paths should still work
            ("/?query=test", true),
            ("///", true),
            ("/v1/something////", false),
            ("/v1/something/", false),
            ("//v1//something", true),
            ("/v1/", true),
            ("/v1", false),
            ("/v1////", true),
            ("//v1//", true),
            ("///v1", false),
        ];

        for (path, success) in tests {
            let req = TestRequest::with_uri(path).to_request();
            let res = call_service(&mut app, req).await;
            assert_eq!(res.status().is_success(), success);
        }
    }

    #[actix_rt::test]
    async fn test_in_place_normalization() {
        let srv = |req: ServiceRequest| {
            assert_eq!("/v1/something/", req.path());
            ok(req.into_response(HttpResponse::Ok().finish()))
        };

        let mut normalize = NormalizePath::default()
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let req = TestRequest::with_uri("/v1//something////").to_srv_request();
        let res = normalize.call(req).await.unwrap();
        assert!(res.status().is_success());

        let req2 = TestRequest::with_uri("///v1/something").to_srv_request();
        let res2 = normalize.call(req2).await.unwrap();
        assert!(res2.status().is_success());

        let req3 = TestRequest::with_uri("//v1///something").to_srv_request();
        let res3 = normalize.call(req3).await.unwrap();
        assert!(res3.status().is_success());

        let req4 = TestRequest::with_uri("/v1//something").to_srv_request();
        let res4 = normalize.call(req4).await.unwrap();
        assert!(res4.status().is_success());
    }

    #[actix_rt::test]
    async fn should_normalize_nothing() {
        const URI: &str = "/v1/something/";

        let srv = |req: ServiceRequest| {
            assert_eq!(URI, req.path());
            ok(req.into_response(HttpResponse::Ok().finish()))
        };

        let mut normalize = NormalizePath::default()
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let req = TestRequest::with_uri(URI).to_srv_request();
        let res = normalize.call(req).await.unwrap();
        assert!(res.status().is_success());
    }

    #[actix_rt::test]
    async fn should_normalize_notrail() {
        const URI: &str = "/v1/something";

        let srv = |req: ServiceRequest| {
            assert_eq!(URI.to_string() + "/", req.path());
            ok(req.into_response(HttpResponse::Ok().finish()))
        };

        let mut normalize = NormalizePath::default()
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let req = TestRequest::with_uri(URI).to_srv_request();
        let res = normalize.call(req).await.unwrap();
        assert!(res.status().is_success());
    }
}
