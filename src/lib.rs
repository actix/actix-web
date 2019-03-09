#![allow(clippy::type_complexity)]

mod app;
mod app_service;
pub mod blocking;
mod config;
pub mod error;
mod extract;
pub mod guard;
mod handler;
mod info;
pub mod middleware;
mod request;
mod resource;
mod responder;
mod rmap;
mod route;
mod scope;
mod server;
mod service;
mod state;
pub mod test;

#[allow(unused_imports)]
#[macro_use]
extern crate actix_web_codegen;

#[doc(hidden)]
pub use actix_web_codegen::*;

// re-export for convenience
pub use actix_http::Response as HttpResponse;
pub use actix_http::{http, Error, HttpMessage, ResponseError, Result};

pub use crate::app::App;
pub use crate::extract::FromRequest;
pub use crate::request::HttpRequest;
pub use crate::resource::Resource;
pub use crate::responder::{Either, Responder};
pub use crate::route::Route;
pub use crate::scope::Scope;
pub use crate::server::HttpServer;
pub use crate::service::{ServiceFromRequest, ServiceRequest, ServiceResponse};

pub mod dev {
    //! The `actix-web` prelude for library developers
    //!
    //! The purpose of this module is to alleviate imports of many common actix
    //! traits by adding a glob import to the top of actix heavy modules:
    //!
    //! ```
    //! # #![allow(unused_imports)]
    //! use actix_web::dev::*;
    //! ```

    pub use crate::app::AppRouter;
    pub use crate::config::{AppConfig, ServiceConfig};
    pub use crate::info::ConnectionInfo;
    pub use crate::rmap::ResourceMap;
    pub use crate::service::HttpServiceFactory;

    pub use actix_http::body::{Body, BodyLength, MessageBody, ResponseBody};
    pub use actix_http::dev::ResponseBuilder as HttpResponseBuilder;
    pub use actix_http::{
        Extensions, Payload, PayloadStream, RequestHead, ResponseHead,
    };
    pub use actix_router::{Path, ResourceDef, ResourcePath, Url};

    pub(crate) fn insert_slash(path: &str) -> String {
        let mut path = path.to_owned();
        if !path.is_empty() && !path.starts_with('/') {
            path.insert(0, '/');
        };
        path
    }
}

pub mod web {
    use actix_http::{http::Method, Error, Response};
    use futures::IntoFuture;

    pub use actix_http::Response as HttpResponse;
    pub use bytes::{Bytes, BytesMut};

    use crate::blocking::CpuFuture;
    use crate::extract::FromRequest;
    use crate::handler::{AsyncFactory, Factory};
    use crate::resource::Resource;
    use crate::responder::Responder;
    use crate::route::Route;
    use crate::scope::Scope;

    pub use crate::extract::{Form, Json, Path, Payload, Query};
    pub use crate::extract::{FormConfig, JsonConfig, PayloadConfig};
    pub use crate::request::HttpRequest;
    pub use crate::state::State;

    /// Create resource for a specific path.
    ///
    /// Resources may have variable path segments. For example, a
    /// resource with the path `/a/{name}/c` would match all incoming
    /// requests with paths such as `/a/b/c`, `/a/1/c`, or `/a/etc/c`.
    ///
    /// A variable segment is specified in the form `{identifier}`,
    /// where the identifier can be used later in a request handler to
    /// access the matched value for that segment. This is done by
    /// looking up the identifier in the `Params` object returned by
    /// `HttpRequest.match_info()` method.
    ///
    /// By default, each segment matches the regular expression `[^{}/]+`.
    ///
    /// You can also specify a custom regex in the form `{identifier:regex}`:
    ///
    /// For instance, to route `GET`-requests on any route matching
    /// `/users/{userid}/{friend}` and store `userid` and `friend` in
    /// the exposed `Params` object:
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{web, http, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::resource("/users/{userid}/{friend}")
    ///             .route(web::get().to(|| HttpResponse::Ok()))
    ///             .route(web::head().to(|| HttpResponse::MethodNotAllowed()))
    ///     );
    /// }
    /// ```
    pub fn resource<P: 'static>(path: &str) -> Resource<P> {
        Resource::new(path)
    }

    /// Configure scope for common root path.
    ///
    /// Scopes collect multiple paths under a common path prefix.
    /// Scope path can contain variable path segments as resources.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{web, App, HttpRequest, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::scope("/{project_id}")
    ///             .service(web::resource("/path1").to(|| HttpResponse::Ok()))
    ///             .service(web::resource("/path2").to(|| HttpResponse::Ok()))
    ///             .service(web::resource("/path3").to(|| HttpResponse::MethodNotAllowed()))
    ///     );
    /// }
    /// ```
    ///
    /// In the above example, three routes get added:
    ///  * /{project_id}/path1
    ///  * /{project_id}/path2
    ///  * /{project_id}/path3
    ///
    pub fn scope<P: 'static>(path: &str) -> Scope<P> {
        Scope::new(path)
    }

    /// Create *route* without configuration.
    pub fn route<P: 'static>() -> Route<P> {
        Route::new()
    }

    /// Create *route* with `GET` method guard.
    pub fn get<P: 'static>() -> Route<P> {
        Route::new().method(Method::GET)
    }

    /// Create *route* with `POST` method guard.
    pub fn post<P: 'static>() -> Route<P> {
        Route::new().method(Method::POST)
    }

    /// Create *route* with `PUT` method guard.
    pub fn put<P: 'static>() -> Route<P> {
        Route::new().method(Method::PUT)
    }

    /// Create *route* with `PATCH` method guard.
    pub fn patch<P: 'static>() -> Route<P> {
        Route::new().method(Method::PATCH)
    }

    /// Create *route* with `DELETE` method guard.
    pub fn delete<P: 'static>() -> Route<P> {
        Route::new().method(Method::DELETE)
    }

    /// Create *route* with `HEAD` method guard.
    pub fn head<P: 'static>() -> Route<P> {
        Route::new().method(Method::HEAD)
    }

    /// Create *route* and add method guard.
    pub fn method<P: 'static>(method: Method) -> Route<P> {
        Route::new().method(method)
    }

    /// Create a new route and add handler.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// fn index() -> HttpResponse {
    ///    unimplemented!()
    /// }
    ///
    /// App::new().service(
    ///     web::resource("/").route(
    ///         web::to(index))
    /// );
    /// ```
    pub fn to<F, I, R, P: 'static>(handler: F) -> Route<P>
    where
        F: Factory<I, R> + 'static,
        I: FromRequest<P> + 'static,
        R: Responder + 'static,
    {
        Route::new().to(handler)
    }

    /// Create a new route and add async handler.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpResponse, Error};
    ///
    /// fn index() -> impl futures::Future<Item=HttpResponse, Error=Error> {
    ///     futures::future::ok(HttpResponse::Ok().finish())
    /// }
    ///
    /// App::new().service(web::resource("/").route(
    ///     web::to_async(index))
    /// );
    /// ```
    pub fn to_async<F, I, R, P: 'static>(handler: F) -> Route<P>
    where
        F: AsyncFactory<I, R>,
        I: FromRequest<P> + 'static,
        R: IntoFuture + 'static,
        R::Item: Into<Response>,
        R::Error: Into<Error>,
    {
        Route::new().to_async(handler)
    }

    /// Execute blocking function on a thread pool, returns future that resolves
    /// to result of the function execution.
    pub fn block<F, I, E>(f: F) -> CpuFuture<I, E>
    where
        F: FnOnce() -> Result<I, E> + Send + 'static,
        I: Send + 'static,
        E: Send + std::fmt::Debug + 'static,
    {
        crate::blocking::run(f)
    }
}
