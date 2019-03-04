#![allow(clippy::type_complexity)]

mod app;
pub mod extract;
mod handler;
// mod info;
pub mod blocking;
pub mod guard;
pub mod middleware;
mod request;
mod resource;
mod responder;
mod route;
mod scope;
mod service;
mod state;
pub mod test;

// re-export for convenience
pub use actix_http::Response as HttpResponse;
pub use actix_http::{error, http, Error, HttpMessage, ResponseError, Result};

pub use crate::app::{App, AppRouter};
pub use crate::extract::{FromRequest, Json};
pub use crate::request::HttpRequest;
pub use crate::resource::Resource;
pub use crate::responder::{Either, Responder};
pub use crate::route::Route;
pub use crate::scope::Scope;
pub use crate::service::{ServiceFromRequest, ServiceRequest, ServiceResponse};
pub use crate::state::State;

pub mod web {
    use actix_http::{http::Method, Error, Response};
    use futures::IntoFuture;

    use crate::extract::FromRequest;
    use crate::handler::{AsyncFactory, Factory};
    use crate::responder::Responder;
    use crate::Route;

    /// Create **route** without configuration.
    pub fn route<P: 'static>() -> Route<P> {
        Route::new()
    }

    /// Create **route** with `GET` method guard.
    pub fn get<P: 'static>() -> Route<P> {
        Route::get()
    }

    /// Create **route** with `POST` method guard.
    pub fn post<P: 'static>() -> Route<P> {
        Route::post()
    }

    /// Create **route** with `PUT` method guard.
    pub fn put<P: 'static>() -> Route<P> {
        Route::put()
    }

    /// Create **route** with `DELETE` method guard.
    pub fn delete<P: 'static>() -> Route<P> {
        Route::delete()
    }

    /// Create **route** with `HEAD` method guard.
    pub fn head<P: 'static>() -> Route<P> {
        Route::new().method(Method::HEAD)
    }

    /// Create **route** and add method guard.
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
    /// App::new().resource("/", |r| r.route(web::to(index)));
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
    /// App::new().resource("/", |r| r.route(web::to_async(index)));
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
}
