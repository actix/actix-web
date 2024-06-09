//! A collection of common middleware.
//!
//! # What Is Middleware?
//!
//! Actix Web's middleware system allows us to add additional behavior to request/response
//! processing. Middleware can hook into incoming request and outgoing response processes, enabling
//! us to modify requests and responses as well as halt request processing to return a response
//! early.
//!
//! Typically, middleware is involved in the following actions:
//!
//! - Pre-process the request (e.g., [normalizing paths](NormalizePath))
//! - Post-process a response (e.g., [logging][Logger])
//! - Modify application state (through [`ServiceRequest`][crate::dev::ServiceRequest])
//! - Access external services (e.g., [sessions](https://docs.rs/actix-session), etc.)
//!
//! Middleware is registered for each [`App`], [`Scope`](crate::Scope), or
//! [`Resource`](crate::Resource) and executed in opposite order as registration. In general, a
//! middleware is a pair of types that implements the [`Service`] trait and [`Transform`] trait,
//! respectively. The [`new_transform`] and [`call`] methods must return a [`Future`], though it
//! can often be [an immediately-ready one](actix_utils::future::Ready).
//!
//! # Ordering
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
//! # type MiddlewareA = middleware::Compress;
//! # type MiddlewareB = middleware::Compress;
//! # type MiddlewareC = middleware::Compress;
//! let app = App::new()
//!     .wrap(MiddlewareA::default())
//!     .wrap(MiddlewareB::default())
//!     .wrap(MiddlewareC::default())
//!     .service(service);
//! # }
//! ```
//!
//! ```plain
//!                   Request
//!                      ⭣
//! ╭────────────────────┼────╮
//! │ MiddlewareC        │    │
//! │ ╭──────────────────┼───╮│
//! │ │ MiddlewareB      │   ││
//! │ │ ╭────────────────┼──╮││
//! │ │ │ MiddlewareA    │  │││
//! │ │ │ ╭──────────────┼─╮│││
//! │ │ │ │ ExtractorA   │ ││││
//! │ │ │ ├┈┈┈┈┈┈┈┈┈┈┈┈┈┈┼┈┤│││
//! │ │ │ │ ExtractorB   │ ││││
//! │ │ │ ├┈┈┈┈┈┈┈┈┈┈┈┈┈┈┼┈┤│││
//! │ │ │ │ service      │ ││││
//! │ │ │ ╰──────────────┼─╯│││
//! │ │ ╰────────────────┼──╯││
//! │ ╰──────────────────┼───╯│
//! ╰────────────────────┼────╯
//!                      ⭣
//!                   Response
//! ```
//! The request _first_ gets processed by the middleware specified _last_ - `MiddlewareC`. It passes
//! the request (modified a modified one) to the next middleware - `MiddlewareB` - _or_ directly
//! responds to the request (e.g. when the request was invalid or an error occurred). `MiddlewareB`
//! processes the request as well and passes it to `MiddlewareA`, which then passes it to the
//! [`Service`]. In the [`Service`], the extractors will run first. They don't pass the request on,
//! but only view it (see [`FromRequest`]). After the [`Service`] responds to the request, the
//! response is passed back through `MiddlewareA`, `MiddlewareB`, and `MiddlewareC`.
//!
//! As you register middleware using [`wrap`][crate::App::wrap] and [`wrap_fn`][crate::App::wrap_fn]
//! in the [`App`] builder, imagine wrapping layers around an inner [`App`]. The first middleware
//! layer exposed to a Request is the outermost layer (i.e., the _last_ registered in the builder
//! chain, in the example above: `MiddlewareC`). Consequently, the _first_ middleware registered in
//! the builder chain is the _last_ to start executing during request processing (`MiddlewareA`).
//! Ordering is less obvious when wrapped services also have middleware applied. In this case,
//! middleware are run in reverse order for [`App`] _and then_ in reverse order for the wrapped
//! service.
//!
//! # Middleware Traits
//!
//! ## `Transform<S, Req>`
//!
//! The [`Transform`] trait is the builder for the actual [`Service`]s that handle the requests. All
//! the middleware you pass to the `wrap` methods implement this trait. During construction, each
//! thread assembles a chain of [`Service`]s by calling [`new_transform`] and passing the next
//! [`Service`] (`S`) in the chain. The created [`Service`] handles requests of type `Req`.
//!
//! In the example from the [ordering](#ordering) section, the chain would be:
//!
//! ```plain
//! MiddlewareCService {
//!     next: MiddlewareBService {
//!         next: MiddlewareAService { ... }
//!     }
//! }
//! ```
//!
//! ## `Service<Req>`
//!
//! A [`Service`] `S` represents an asynchronous operation that turns a request of type `Req` into a
//! response of type [`S::Response`](crate::dev::Service::Response) or an error of type
//! [`S::Error`](crate::dev::Service::Error). You can think of the service of being roughly:
//!
//! ```ignore
//! async fn(&self, req: Req) -> Result<S::Response, S::Error>
//! ```
//!
//! In most cases the [`Service`] implementation will, at some point, call the wrapped [`Service`]
//! in its [`call`] implementation.
//!
//! Note that the [`Service`]s created by [`new_transform`] don't need to be [`Send`] or [`Sync`].
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
//! // See: futures_util::future::LocalBoxFuture
//! type LocalBoxFuture<T> = Pin<Box<dyn Future<Output = T> + 'static>>;
//!
//! // `S`: type of the wrapped service
//! // `B`: type of the body - try to be generic over the body where possible
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
//! # Simpler Middleware
//!
//! In many cases, you _can_ actually use an async function via a helper that will provide a more
//! natural flow for your behavior.
//!
//! The experimental `actix_web_lab` crate provides a [`from_fn`][lab_from_fn] utility which allows
//! an async fn to be wrapped and used in the same way as other middleware. See the
//! [`from_fn`][lab_from_fn] docs for more info and examples of it's use.
//!
//! While [`from_fn`][lab_from_fn] is experimental currently, it's likely this helper will graduate
//! to Actix Web in some form, so feedback is appreciated.
//!
//! [`Future`]: std::future::Future
//! [`App`]: crate::App
//! [`FromRequest`]: crate::FromRequest
//! [`Service`]: crate::dev::Service
//! [`Transform`]: crate::dev::Transform
//! [`call`]: crate::dev::Service::call()
//! [`new_transform`]: crate::dev::Transform::new_transform()
//! [lab_from_fn]: https://docs.rs/actix-web-lab/latest/actix_web_lab/middleware/fn.from_fn.html

mod compat;
#[cfg(feature = "__compress")]
mod compress;
mod condition;
mod default_headers;
mod err_handlers;
mod identity;
mod logger;
mod normalize;

#[cfg(feature = "__compress")]
pub use self::compress::Compress;
pub use self::{
    compat::Compat,
    condition::Condition,
    default_headers::DefaultHeaders,
    err_handlers::{ErrorHandlerResponse, ErrorHandlers},
    identity::Identity,
    logger::Logger,
    normalize::{NormalizePath, TrailingSlash},
};

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
