//! For middleware documentation, see [`NormalizePath`].

use actix_http::uri::{PathAndQuery, Uri};
use actix_service::{Service, Transform};
use actix_utils::future::{ready, Ready};
use bytes::Bytes;
#[cfg(feature = "unicode")]
use regex::Regex;
#[cfg(not(feature = "unicode"))]
use regex_lite::Regex;

use crate::{
    service::{ServiceRequest, ServiceResponse},
    Error,
};

/// Determines the behavior of the [`NormalizePath`] middleware.
///
/// The default is `TrailingSlash::Trim`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default)]
pub enum TrailingSlash {
    /// Trim trailing slashes from the end of the path.
    ///
    /// Using this will require all routes to omit trailing slashes for them to be accessible.
    #[default]
    Trim,

    /// Only merge any present multiple trailing slashes.
    ///
    /// This option provides the best compatibility with behavior in actix-web v2.0.
    MergeOnly,

    /// Always add a trailing slash to the end of the path.
    ///
    /// Using this will require all routes have a trailing slash for them to be accessible.
    Always,
}

/// Middleware for normalizing a request's path so that routes can be matched more flexibly.
///
/// # Normalization Steps
/// - Merges consecutive slashes into one. (For example, `/path//one` always becomes `/path/one`.)
/// - Appends a trailing slash if one is not present, removes one if present, or keeps trailing
///   slashes as-is, depending on which [`TrailingSlash`] variant is supplied
///   to [`new`](NormalizePath::new()).
///
/// # Default Behavior
/// The default constructor chooses to strip trailing slashes from the end of paths with them
/// ([`TrailingSlash::Trim`]). The implication is that route definitions should be defined without
/// trailing slashes or else they will be inaccessible (or vice versa when using the
/// `TrailingSlash::Always` behavior), as shown in the example tests below.
///
/// # Examples
/// ```
/// use actix_web::{web, middleware, App};
///
/// # actix_web::rt::System::new().block_on(async {
/// let app = App::new()
///     .wrap(middleware::NormalizePath::trim())
///     .route("/test", web::get().to(|| async { "test" }))
///     .route("/unmatchable/", web::get().to(|| async { "unmatchable" }));
///
/// use actix_web::http::StatusCode;
/// use actix_web::test::{call_service, init_service, TestRequest};
///
/// let app = init_service(app).await;
///
/// let req = TestRequest::with_uri("/test").to_request();
/// let res = call_service(&app, req).await;
/// assert_eq!(res.status(), StatusCode::OK);
///
/// let req = TestRequest::with_uri("/test/").to_request();
/// let res = call_service(&app, req).await;
/// assert_eq!(res.status(), StatusCode::OK);
///
/// let req = TestRequest::with_uri("/unmatchable").to_request();
/// let res = call_service(&app, req).await;
/// assert_eq!(res.status(), StatusCode::NOT_FOUND);
///
/// let req = TestRequest::with_uri("/unmatchable/").to_request();
/// let res = call_service(&app, req).await;
/// assert_eq!(res.status(), StatusCode::NOT_FOUND);
/// # })
/// ```
#[derive(Debug, Clone, Copy)]
pub struct NormalizePath(TrailingSlash);

impl Default for NormalizePath {
    fn default() -> Self {
        log::warn!(
            "`NormalizePath::default()` is deprecated. The default trailing slash behavior changed \
            in v4 from `Always` to `Trim`. Update your call to `NormalizePath::new(...)`."
        );

        Self(TrailingSlash::Trim)
    }
}

impl NormalizePath {
    /// Create new `NormalizePath` middleware with the specified trailing slash style.
    pub fn new(trailing_slash_style: TrailingSlash) -> Self {
        Self(trailing_slash_style)
    }

    /// Constructs a new `NormalizePath` middleware with [trim](TrailingSlash::Trim) semantics.
    ///
    /// Use this instead of `NormalizePath::default()` to avoid deprecation warning.
    pub fn trim() -> Self {
        Self::new(TrailingSlash::Trim)
    }
}

impl<S, B> Transform<S, ServiceRequest> for NormalizePath
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = NormalizePathNormalization<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(NormalizePathNormalization {
            service,
            merge_slash: Regex::new("//+").unwrap(),
            trailing_slash_behavior: self.0,
        }))
    }
}

pub struct NormalizePathNormalization<S> {
    service: S,
    merge_slash: Regex,
    trailing_slash_behavior: TrailingSlash,
}

impl<S, B> Service<ServiceRequest> for NormalizePathNormalization<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = S::Future;

    actix_service::forward_ready!(service);

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let head = req.head_mut();

        let original_path = head.uri.path();

        // An empty path here means that the URI has no valid path. We skip normalization in this
        // case, because adding a path can make the URI invalid
        if !original_path.is_empty() {
            // Either adds a string to the end (duplicates will be removed anyways) or trims all
            // slashes from the end
            let path = match self.trailing_slash_behavior {
                TrailingSlash::Always => format!("{}/", original_path),
                TrailingSlash::MergeOnly => original_path.to_string(),
                TrailingSlash::Trim => original_path.trim_end_matches('/').to_string(),
            };

            // normalize multiple /'s to one /
            let path = self.merge_slash.replace_all(&path, "/");

            // Ensure root paths are still resolvable. If resulting path is blank after previous
            // step it means the path was one or more slashes. Reduce to single slash.
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
                let query = parts.path_and_query.as_ref().and_then(|pq| pq.query());

                let path = match query {
                    Some(q) => Bytes::from(format!("{}?{}", path, q)),
                    None => Bytes::copy_from_slice(path.as_bytes()),
                };
                parts.path_and_query = Some(PathAndQuery::from_maybe_shared(path).unwrap());

                let uri = Uri::from_parts(parts).unwrap();
                req.match_info_mut().get_mut().update(&uri);
                req.head_mut().uri = uri;
            }
        }
        self.service.call(req)
    }
}

#[cfg(test)]
mod tests {
    use actix_http::StatusCode;
    use actix_service::IntoService;

    use super::*;
    use crate::{
        guard::fn_guard,
        test::{call_service, init_service, TestRequest},
        web, App, HttpResponse,
    };

    #[actix_rt::test]
    async fn test_wrap() {
        let app = init_service(
            App::new()
                .wrap(NormalizePath::default())
                .service(web::resource("/").to(HttpResponse::Ok))
                .service(web::resource("/v1/something").to(HttpResponse::Ok))
                .service(
                    web::resource("/v2/something")
                        .guard(fn_guard(|ctx| ctx.head().uri.query() == Some("query=test")))
                        .to(HttpResponse::Ok),
                ),
        )
        .await;

        let test_uris = vec![
            "/",
            "/?query=test",
            "///",
            "/v1//something",
            "/v1//something////",
            "//v1/something",
            "//v1//////something",
            "/v2//something?query=test",
            "/v2//something////?query=test",
            "//v2/something?query=test",
            "//v2//////something?query=test",
        ];

        for uri in test_uris {
            let req = TestRequest::with_uri(uri).to_request();
            let res = call_service(&app, req).await;
            assert!(res.status().is_success(), "Failed uri: {}", uri);
        }
    }

    #[actix_rt::test]
    async fn trim_trailing_slashes() {
        let app = init_service(
            App::new()
                .wrap(NormalizePath(TrailingSlash::Trim))
                .service(web::resource("/").to(HttpResponse::Ok))
                .service(web::resource("/v1/something").to(HttpResponse::Ok))
                .service(
                    web::resource("/v2/something")
                        .guard(fn_guard(|ctx| ctx.head().uri.query() == Some("query=test")))
                        .to(HttpResponse::Ok),
                ),
        )
        .await;

        let test_uris = vec![
            "/",
            "///",
            "/v1/something",
            "/v1/something/",
            "/v1/something////",
            "//v1//something",
            "//v1//something//",
            "/v2/something?query=test",
            "/v2/something/?query=test",
            "/v2/something////?query=test",
            "//v2//something?query=test",
            "//v2//something//?query=test",
        ];

        for uri in test_uris {
            let req = TestRequest::with_uri(uri).to_request();
            let res = call_service(&app, req).await;
            assert!(res.status().is_success(), "Failed uri: {}", uri);
        }
    }

    #[actix_rt::test]
    async fn trim_root_trailing_slashes_with_query() {
        let app = init_service(
            App::new().wrap(NormalizePath(TrailingSlash::Trim)).service(
                web::resource("/")
                    .guard(fn_guard(|ctx| ctx.head().uri.query() == Some("query=test")))
                    .to(HttpResponse::Ok),
            ),
        )
        .await;

        let test_uris = vec!["/?query=test", "//?query=test", "///?query=test"];

        for uri in test_uris {
            let req = TestRequest::with_uri(uri).to_request();
            let res = call_service(&app, req).await;
            assert!(res.status().is_success(), "Failed uri: {}", uri);
        }
    }

    #[actix_rt::test]
    async fn ensure_trailing_slash() {
        let app = init_service(
            App::new()
                .wrap(NormalizePath(TrailingSlash::Always))
                .service(web::resource("/").to(HttpResponse::Ok))
                .service(web::resource("/v1/something/").to(HttpResponse::Ok))
                .service(
                    web::resource("/v2/something/")
                        .guard(fn_guard(|ctx| ctx.head().uri.query() == Some("query=test")))
                        .to(HttpResponse::Ok),
                ),
        )
        .await;

        let test_uris = vec![
            "/",
            "///",
            "/v1/something",
            "/v1/something/",
            "/v1/something////",
            "//v1//something",
            "//v1//something//",
            "/v2/something?query=test",
            "/v2/something/?query=test",
            "/v2/something////?query=test",
            "//v2//something?query=test",
            "//v2//something//?query=test",
        ];

        for uri in test_uris {
            let req = TestRequest::with_uri(uri).to_request();
            let res = call_service(&app, req).await;
            assert!(res.status().is_success(), "Failed uri: {}", uri);
        }
    }

    #[actix_rt::test]
    async fn ensure_root_trailing_slash_with_query() {
        let app = init_service(
            App::new()
                .wrap(NormalizePath(TrailingSlash::Always))
                .service(
                    web::resource("/")
                        .guard(fn_guard(|ctx| ctx.head().uri.query() == Some("query=test")))
                        .to(HttpResponse::Ok),
                ),
        )
        .await;

        let test_uris = vec!["/?query=test", "//?query=test", "///?query=test"];

        for uri in test_uris {
            let req = TestRequest::with_uri(uri).to_request();
            let res = call_service(&app, req).await;
            assert!(res.status().is_success(), "Failed uri: {}", uri);
        }
    }

    #[actix_rt::test]
    async fn keep_trailing_slash_unchanged() {
        let app = init_service(
            App::new()
                .wrap(NormalizePath(TrailingSlash::MergeOnly))
                .service(web::resource("/").to(HttpResponse::Ok))
                .service(web::resource("/v1/something").to(HttpResponse::Ok))
                .service(web::resource("/v1/").to(HttpResponse::Ok))
                .service(
                    web::resource("/v2/something")
                        .guard(fn_guard(|ctx| ctx.head().uri.query() == Some("query=test")))
                        .to(HttpResponse::Ok),
                ),
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
            ("/v2/something?query=test", true),
            ("/v2/something/?query=test", false),
            ("/v2/something//?query=test", false),
            ("//v2//something?query=test", true),
        ];

        for (uri, success) in tests {
            let req = TestRequest::with_uri(uri).to_request();
            let res = call_service(&app, req).await;
            assert_eq!(res.status().is_success(), success, "Failed uri: {}", uri);
        }
    }

    #[actix_rt::test]
    async fn no_path() {
        let app = init_service(
            App::new()
                .wrap(NormalizePath::default())
                .service(web::resource("/").to(HttpResponse::Ok)),
        )
        .await;

        // This URI will be interpreted as an authority form, i.e. there is no path nor scheme
        // (https://datatracker.ietf.org/doc/html/rfc7230#section-5.3.3)
        let req = TestRequest::with_uri("eh").to_request();
        let res = call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_in_place_normalization() {
        let srv = |req: ServiceRequest| {
            assert_eq!("/v1/something", req.path());
            ready(Ok(req.into_response(HttpResponse::Ok().finish())))
        };

        let normalize = NormalizePath::default()
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let test_uris = vec![
            "/v1//something////",
            "///v1/something",
            "//v1///something",
            "/v1//something",
        ];

        for uri in test_uris {
            let req = TestRequest::with_uri(uri).to_srv_request();
            let res = normalize.call(req).await.unwrap();
            assert!(res.status().is_success(), "Failed uri: {}", uri);
        }
    }

    #[actix_rt::test]
    async fn should_normalize_nothing() {
        const URI: &str = "/v1/something";

        let srv = |req: ServiceRequest| {
            assert_eq!(URI, req.path());
            ready(Ok(req.into_response(HttpResponse::Ok().finish())))
        };

        let normalize = NormalizePath::default()
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let req = TestRequest::with_uri(URI).to_srv_request();
        let res = normalize.call(req).await.unwrap();
        assert!(res.status().is_success());
    }

    #[actix_rt::test]
    async fn should_normalize_no_trail() {
        let srv = |req: ServiceRequest| {
            assert_eq!("/v1/something", req.path());
            ready(Ok(req.into_response(HttpResponse::Ok().finish())))
        };

        let normalize = NormalizePath::default()
            .new_transform(srv.into_service())
            .await
            .unwrap();

        let req = TestRequest::with_uri("/v1/something/").to_srv_request();
        let res = normalize.call(req).await.unwrap();
        assert!(res.status().is_success());
    }
}
