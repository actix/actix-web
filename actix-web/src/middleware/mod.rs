//! A collection of common middleware.
//!
//! # Introduction
//!
//! Actix Web's middleware system allows us to add additional behavior to request/response processing.
//! Middleware can hook into incoming request and outgoing response processes,
//! enabling us to modify requests and responses as well as halt request processing to return a response early.
//!
//! Typically, middleware is involved in the following actions:
//!
//! * Pre-process the request
//! * Post-process a response
//! * Modify application state (through [`ServiceRequest`][crate::dev::ServiceRequest])
//! * Access external services ([redis](https://docs.rs/actix-redis), [logging][Logger], [sessions](https://docs.rs/actix-session))
//!
//! Middleware is registered for each [`App`][crate::App], [`Scope`][crate::Scope], or [`Resource`][crate::Resource]
//! and executed in opposite order as registration.
//! In general, a middleware is a type that implements the [`Service`][Service] trait and [`Transform`][Transform] trait.
//! Each method in the traits has a default implementation. Each method can return a result immediately or a [`Future`][std::future::Future].
//!
//! ## Order
//!
//! ```
//! # use actix_web::{web, middleware, get, App, Responder};
//! #
//! # // some basic types to make sure this compiles
//! # type ExtractorA = web::Json<String>;
//! # type ExtractorB = ExtractorA;
//! #[get("/")]
//! async fn service(a: ExtractorA, b: ExtractorB) -> impl Responder { "Hello, World!" }
//!
//! # fn main() {
//! # // These aren't snake_case, because they are supposed to be unit structs.
//! # let MiddlewareA = middleware::Compress::default();
//! # let MiddlewareB = middleware::Compress::default();
//! # let MiddlewareC = middleware::Compress::default();
//! let app = App::new()
//!     .wrap(MiddlewareA)
//!     .wrap(MiddlewareB)
//!     .wrap(MiddlewareC)
//!     .service(service);
//! # }
//! ```
//!
//! ```text
//!                   Request
//!                      ⭣
//! ╭────────────────────┼───╮
//! │ MiddlewareC        │   │
//! │ ╭──────────────────┼──╮│
//! │ │ MiddlewareB      │  ││
//! │ │ ╭────────────────┼─╮││
//! │ │ │ MiddlewareA    │ │││
//! │ │ │ ╭──────────────┼╮│││
//! │ │ │ │ ExtractorA   │││││
//! │ │ │ ├┈┈┈┈┈┈┈┈┈┈┈┈┈┈┼┤│││
//! │ │ │ │ ExtractorB   │││││
//! │ │ │ ├┈┈┈┈┈┈┈┈┈┈┈┈┈┈┼┤│││
//! │ │ │ │ service      │││││
//! │ │ │ ╰──────────────┼╯│││
//! │ │ ╰────────────────┼─╯││
//! │ ╰──────────────────┼──╯│
//! ╰────────────────────┼───╯
//!                      ⭣
//!                   Response
//! ```
//! The request _first_ gets processed by the middleware specified _last_ - `MiddlewareC`.
//! It passes the request (or a modified one) to the next middleware - `MiddlewareB` -
//! _or_ directly responds to the request (e.g. when the request was invalid or an error occurred).
//! `MiddlewareB` processes the request as well and passes it to `MiddlewareA`, which then passes it to the [`Service`][Service].
//! In the [`Service`][Service], the extractors will run first. They don't pass the request on, but only view it (see [`FromRequest`][crate::FromRequest]).
//! After the [`Service`][Service] responds to the request, the response it passed back through `MiddlewareA`, `MiddlewareB`, and `MiddlewareC`.
//!
//! As you register middleware using [`wrap`][crate::App::wrap] and [`wrap_fn`][crate::App::wrap_fn] in the [`App`][crate::App] builder,
//! imagine wrapping layers around an inner [`App`][crate::App].
//! The first middleware layer exposed to a Request is the outermost layer (i.e., the *last* registered in
//! the builder chain, in the example above: `MiddlewareC`). Consequently, the *first* middleware registered in the builder chain is
//! the *last* to start executing during request processing (`MiddlewareA`).
//! Ordering is less obvious when wrapped services also have middleware applied. In this case,
//! middlewares are run in reverse order for [`App`][crate::App] _and then_ in reverse order for the
//! wrapped service.
//!
//! # Middleware Traits
//!
//! ## `Transform<S, Req>`
//!
//! The [`Transform`][Transform] trait is the factory for the actual [`Service`][crate::dev::Service]s that handle the requests.
//! All the middleware you pass to the `wrap` methods implement this trait.
//! During construction, each thread assembles a chain of [`Service`][Service]s
//! by calling [`new_transform`][crate::dev::Transform::new_transform] and passing the next [`Service`][Service] (`S`) in the chain.
//! The created [`Service`][Service] handles requests of type `Req`.
//!
//! In the example from the [Order](#Order) section, the chain would be: `MiddlewareCService { next: MiddlewareBService { next: MiddlewareAService {..} } } }`.
//!
//! ## `Service<Req>`
//!
//! A [`Service`][Service] `S` represents an asynchronous operation
//! that turns a request of type `Req` into a response of type [`S::Response`][crate::dev::Service::Response]
//! or an error of type [`S::Error`][crate::dev::Service::Error]. You can think of the service of being a `async fn (&self, req: Req) -> Result<S::Response, S::Error>`.
//!
//! In most cases the [`Service`][Service] implementation will call the next [`Service`][Service] in its [`Future`][std::future::Future] returned by [`call`][crate::dev::Service::call].
//!
//! Note that the [`Service`][Service]s created by [`new_transform`][crate::dev::Transform::new_transform] don't need to be [`Send`][Send] nor [`Sync`][Sync].
//!
//! # Example
//!
//! ```
//! use std::{future::{ready, Ready, Future}, pin::Pin};
//!
//! use actix_web::{
//!     dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
//!     web, Error,
//! #   App
//! };
//!
//! pub struct SayHi;
//!
//! // `S` - type of the next service
//! // `B` - type of response's body
//! impl<S, B> Transform<S, ServiceRequest> for SayHi
//! where
//!     S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
//!     S::Future: 'static,
//!     B: 'static,
//! {
//!     type Response = ServiceResponse<B>;
//!     type Error = Error;
//!     type InitError = ();
//!     type Transform = SayHiMiddleware<S>;
//!     type Future = Ready<Result<Self::Transform, Self::InitError>>;
//!
//!     fn new_transform(&self, service: S) -> Self::Future {
//!         ready(Ok(SayHiMiddleware { service }))
//!     }
//! }
//!
//! pub struct SayHiMiddleware<S> {
//!     /// The next service to call
//!     service: S,
//! }
//!
//! // This future doesn't have the requirement of being `Send`.
//! // See futures_util::future::LocalBoxFuture
//! type LocalBoxFuture<T> = Pin<Box<dyn Future<Output = T> + 'static>>;
//!
//! // `S` - type of the next service
//! // `B` - type of the body - try to be generic over the body where possible
//! impl<S, B> Service<ServiceRequest> for SayHiMiddleware<S>
//! where
//!     S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
//!     S::Future: 'static,
//!     B: 'static,
//! {
//!     type Response = ServiceResponse<B>;
//!     type Error = Error;
//!     type Future = LocalBoxFuture<Result<Self::Response, Self::Error>>;
//!
//!     // This service is ready when its next service is ready
//!     forward_ready!(service);
//!
//!     fn call(&self, req: ServiceRequest) -> Self::Future {
//!         println!("Hi from start. You requested: {}", req.path());
//!
//!         // A more complex middleware, could return an error or an early response here.
//!
//!         let fut = self.service.call(req);
//!
//!         Box::pin(async move {
//!             let res = fut.await?;
//!
//!             println!("Hi from response");
//!             Ok(res)
//!         })
//!     }
//! }
//!
//! # fn main() {
//! let app = App::new()
//!     .wrap(SayHi)
//!     .route("/", web::get().to(|| async { "Hello, middleware!" }));
//! # }
//! ```
//!
//! # Simple Middleware
//!
//! In simple cases, you can use a function instead.
//! You can register these in [`App::wrap_fn`][crate::App::wrap_fn], [`Scope::wrap_fn`][crate::Scope::wrap_fn], and [`Resource::wrap_fn`][crate::Resource::wrap_fn].
//! The [order](#order) remains the same.
//!
//! The middleware from [above](#example) can be written using `wrap_fn`:
//!
//! ```
//! use actix_web::{dev::Service, web, App};
//!
//! # fn main() {
//! let app = App::new()
//!     .wrap_fn(|req, srv| {
//!         println!("Hi from start. You requested: {}", req.path());
//!         let fut = srv.call(req);
//!         async {
//!             let res = fut.await?;
//!             
//!             println!("Hi from response");
//!
//!             Ok(res)
//!         }
//!     })
//!     .route("/", web::get().to(|| async { "Hello, middleware!" }));
//! # }
//! ```
//!
//! [Service]: crate::dev::Service
//! [Transform]: crate::dev::Transform

mod compat;
mod condition;
mod default_headers;
mod err_handlers;
mod logger;
#[cfg(test)]
mod noop;
mod normalize;

#[cfg(test)]
pub(crate) use self::noop::Noop;
pub use self::{
    compat::Compat,
    condition::Condition,
    default_headers::DefaultHeaders,
    err_handlers::{ErrorHandlerResponse, ErrorHandlers},
    logger::Logger,
    normalize::{NormalizePath, TrailingSlash},
};

#[cfg(feature = "__compress")]
mod compress;

#[cfg(feature = "__compress")]
pub use self::compress::Compress;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{http::StatusCode, App};

    #[test]
    fn common_combinations() {
        // ensure there's no reason that the built-in middleware cannot compose

        let _ = App::new()
            .wrap(Compat::new(Logger::default()))
            .wrap(Condition::new(true, DefaultHeaders::new()))
            .wrap(DefaultHeaders::new().add(("X-Test2", "X-Value2")))
            .wrap(ErrorHandlers::new().handler(StatusCode::FORBIDDEN, |res| {
                Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
            }))
            .wrap(Logger::default())
            .wrap(NormalizePath::new(TrailingSlash::Trim));

        let _ = App::new()
            .wrap(NormalizePath::new(TrailingSlash::Trim))
            .wrap(Logger::default())
            .wrap(ErrorHandlers::new().handler(StatusCode::FORBIDDEN, |res| {
                Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
            }))
            .wrap(DefaultHeaders::new().add(("X-Test2", "X-Value2")))
            .wrap(Condition::new(true, DefaultHeaders::new()))
            .wrap(Compat::new(Logger::default()));

        #[cfg(feature = "__compress")]
        {
            let _ = App::new().wrap(Compress::default()).wrap(Logger::default());
            let _ = App::new().wrap(Logger::default()).wrap(Compress::default());
            let _ = App::new().wrap(Compat::new(Compress::default()));
            let _ = App::new().wrap(Condition::new(true, Compat::new(Compress::default())));
        }
    }
}
