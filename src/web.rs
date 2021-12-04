//! Essentials helper functions and types for application registration.

use std::{error::Error as StdError, future::Future};

use actix_http::http::Method;
use actix_router::IntoPatterns;
pub use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::{
    body::MessageBody, error::BlockingError, extract::FromRequest, handler::Handler,
    resource::Resource, responder::Responder, route::Route, scope::Scope, service::WebService,
};

pub use crate::config::ServiceConfig;
pub use crate::data::Data;
pub use crate::request::HttpRequest;
pub use crate::request_data::ReqData;
pub use crate::response::HttpResponse;
pub use crate::types::*;

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
/// # Examples
/// In this example, three routes are set up (and will handle any method):
///  * `/{project_id}/path1`
///  * `/{project_id}/path2`
///  * `/{project_id}/path3`
///
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
pub fn scope(path: &str) -> Scope {
    Scope::new(path)
}

/// Creates a new un-configured route.
pub fn route() -> Route {
    Route::new()
}

macro_rules! method_route {
    ($method_fn:ident, $method_const:ident) => {
        paste::paste! {
            #[doc = " Creates a new route with `" $method_const "` method guard."]
            ///
            /// # Examples
            #[doc = " In this example, one `" $method_const " /{project_id}` route is set up:"]
            /// ```
            /// use actix_web::{web, App, HttpResponse};
            ///
            /// let app = App::new().service(
            ///     web::resource("/{project_id}")
            #[doc = "         .route(web::" $method_fn "().to(|| HttpResponse::Ok()))"]
            ///
            /// );
            /// ```
            pub fn $method_fn() -> Route {
                method(Method::$method_const)
            }
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
pub fn to<F, I, R>(handler: F) -> Route
where
    F: Handler<I, R>,
    I: FromRequest + 'static,
    R: Future + 'static,
    R::Output: Responder + 'static,
    <R::Output as Responder>::Body: MessageBody + 'static,
    <<R::Output as Responder>::Body as MessageBody>::Error: Into<Box<dyn StdError + 'static>>,
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
