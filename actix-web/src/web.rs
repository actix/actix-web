//! Essentials helper functions and types for application registration.
//!
//! # Request Extractors
//! - [`Data`]: Application data item
//! - [`ReqData`]: Request-local data item
//! - [`Path`]: URL path parameters / dynamic segments
//! - [`Query`]: URL query parameters
//! - [`Header`]: Typed header
//! - [`Json`]: JSON payload
//! - [`Form`]: URL-encoded payload
//! - [`Bytes`]: Raw payload
//!
//! # Responders
//! - [`Json`]: JSON response
//! - [`Form`]: URL-encoded response
//! - [`Bytes`]: Raw bytes response
//! - [`Redirect`](Redirect::to): Convenient redirect responses

use std::{borrow::Cow, future::Future};

use actix_router::IntoPatterns;
pub use bytes::{Buf, BufMut, Bytes, BytesMut};

pub use crate::{
    config::ServiceConfig, data::Data, redirect::Redirect, request_data::ReqData, types::*,
};
use crate::{
    error::BlockingError, http::Method, service::WebService, FromRequest, Handler, Resource,
    Responder, Route, Scope,
};

/// Creates a new resource for a specific path.
///
/// Resources may have dynamic path segments. For example, a resource with the path `/a/{name}/c`
/// would match all incoming requests with paths such as `/a/b/c`, `/a/1/c`, or `/a/etc/c`.
///
/// A dynamic segment is specified in the form `{identifier}`, where the identifier can be used
/// later in a request handler to access the matched value for that segment. This is done by looking
/// up the identifier in the `Path` object returned by [`HttpRequest.match_info()`] method.
///
/// By default, each segment matches the regular expression `[^{}/]+`.
///
/// You can also specify a custom regex in the form `{identifier:regex}`:
///
/// For instance, to route `GET`-requests on any route matching `/users/{userid}/{friend}` and store
/// `userid` and `friend` in the exposed `Path` object:
///
/// # Examples
/// ```
/// use actix_web::{web, App, HttpResponse};
///
/// let app = App::new().service(
///     web::resource("/users/{userid}/{friend}")
///         .route(web::get().to(|| HttpResponse::Ok()))
///         .route(web::head().to(|| HttpResponse::MethodNotAllowed()))
/// );
/// ```
pub fn resource<T: IntoPatterns>(path: T) -> Resource {
    Resource::new(path)
}

/// Creates scope for common path prefix.
///
/// Scopes collect multiple paths under a common path prefix. The scope's path can contain dynamic
/// path segments.
///
/// # Avoid Trailing Slashes
/// Avoid using trailing slashes in the scope prefix (e.g., `web::scope("/scope/")`). It will almost
/// certainly not have the expected behavior. See the [documentation on resource definitions][pat]
/// to understand why this is the case and how to correctly construct scope/prefix definitions.
///
/// # Examples
/// In this example, three routes are set up (and will handle any method):
/// - `/{project_id}/path1`
/// - `/{project_id}/path2`
/// - `/{project_id}/path3`
///
/// # Examples
/// ```
/// use actix_web::{web, App, HttpResponse};
///
/// let app = App::new().service(
///     web::scope("/{project_id}")
///         .service(web::resource("/path1").to(|| HttpResponse::Ok()))
///         .service(web::resource("/path2").to(|| HttpResponse::Ok()))
///         .service(web::resource("/path3").to(|| HttpResponse::MethodNotAllowed()))
/// );
/// ```
///
/// [pat]: crate::dev::ResourceDef#prefix-resources
pub fn scope(path: &str) -> Scope {
    Scope::new(path)
}

/// Creates a new un-configured route.
pub fn route() -> Route {
    Route::new()
}

macro_rules! method_route {
    ($method_fn:ident, $method_const:ident) => {
        #[doc = concat!(" Creates a new route with `", stringify!($method_const), "` method guard.")]
        ///
        /// # Examples
        #[doc = concat!(" In this example, one `", stringify!($method_const), " /{project_id}` route is set up:")]
        /// ```
        /// use actix_web::{web, App, HttpResponse};
        ///
        /// let app = App::new().service(
        ///     web::resource("/{project_id}")
        #[doc = concat!("         .route(web::", stringify!($method_fn), "().to(|| HttpResponse::Ok()))")]
        ///
        /// );
        /// ```
        pub fn $method_fn() -> Route {
            method(Method::$method_const)
        }
    };
}

method_route!(get, GET);
method_route!(post, POST);
method_route!(put, PUT);
method_route!(patch, PATCH);
method_route!(delete, DELETE);
method_route!(head, HEAD);
method_route!(trace, TRACE);

/// Creates a new route with specified method guard.
///
/// # Examples
/// In this example, one `GET /{project_id}` route is set up:
///
/// ```
/// use actix_web::{web, http, App, HttpResponse};
///
/// let app = App::new().service(
///     web::resource("/{project_id}")
///         .route(web::method(http::Method::GET).to(|| HttpResponse::Ok()))
/// );
/// ```
pub fn method(method: Method) -> Route {
    Route::new().method(method)
}

/// Creates a new any-method route with handler.
///
/// ```
/// use actix_web::{web, App, HttpResponse, Responder};
///
/// async fn index() -> impl Responder {
///    HttpResponse::Ok()
/// }
///
/// App::new().service(
///     web::resource("/").route(
///         web::to(index))
/// );
/// ```
pub fn to<F, Args>(handler: F) -> Route
where
    F: Handler<Args>,
    Args: FromRequest + 'static,
    F::Output: Responder + 'static,
{
    Route::new().to(handler)
}

/// Creates a raw service for a specific path.
///
/// ```
/// use actix_web::{dev, web, guard, App, Error, HttpResponse};
///
/// async fn my_service(req: dev::ServiceRequest) -> Result<dev::ServiceResponse, Error> {
///     Ok(req.into_response(HttpResponse::Ok().finish()))
/// }
///
/// let app = App::new().service(
///     web::service("/users/*")
///         .guard(guard::Header("content-type", "text/plain"))
///         .finish(my_service)
/// );
/// ```
pub fn service<T: IntoPatterns>(path: T) -> WebService {
    WebService::new(path)
}

/// Create a relative or absolute redirect.
///
/// See [`Redirect`] docs for usage details.
///
/// # Examples
/// ```
/// use actix_web::{web, App};
///
/// let app = App::new()
///     // the client will resolve this redirect to /api/to-path
///     .service(web::redirect("/api/from-path", "to-path"));
/// ```
pub fn redirect(from: impl Into<Cow<'static, str>>, to: impl Into<Cow<'static, str>>) -> Redirect {
    Redirect::new(from, to)
}

/// Executes blocking function on a thread pool, returns future that resolves to result of the
/// function execution.
pub fn block<F, R>(f: F) -> impl Future<Output = Result<R, BlockingError>>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let fut = actix_rt::task::spawn_blocking(f);
    async { fut.await.map_err(|_| BlockingError) }
}
