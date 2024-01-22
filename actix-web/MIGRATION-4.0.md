# Migrating to 4.0.0

This guide walks you through the process of migrating from v3.x.y to v4.x.y.  
If you are migrating to v4.x.y from an older version of Actix Web (v2.x.y or earlier), check out the other historical migration notes in this folder.

This document is not designed to be exhaustive—it focuses on the most significant changes in v4. You can find an exhaustive changelog in the changelogs for [`actix-web`](./CHANGES.md#400---2022-02-25) and [`actix-http`](../actix-http/CHANGES.md#300---2022-02-25), complete with PR links. If you think there are any changes that deserve to be called out in this document, please open an issue or pull request.

Headings marked with :warning: are **breaking behavioral changes**. They will probably not surface as compile-time errors though automated tests _might_ detect their effects on your app.

## Table of Contents:

- [MSRV](#msrv)
- [Tokio v1 Ecosystem](#tokio-v1-ecosystem)
- [Module Structure](#module-structure)
- [`NormalizePath` Middleware :warning:](#normalizepath-middleware-warning)
- [Server Settings :warning:](#server-settings-warning)
- [`FromRequest` Trait](#fromrequest-trait)
- [Compression Feature Flags](#compression-feature-flags)
- [`web::Path`](#webpath)
- [Rustls Crate Upgrade](#rustls-crate-upgrade)
- [Removed `awc` Client Re-export](#removed-awc-client-re-export)
- [Integration Testing Utils Moved To `actix-test`](#integration-testing-utils-moved-to-actix-test)
- [Header APIs](#header-apis)
- [Response Body Types](#response-body-types)
- [Middleware Trait APIs](#middleware-trait-apis)
- [`Responder` Trait](#responder-trait)
- [`App::data` Deprecation :warning:](#appdata-deprecation-warning)
- [Direct Dependency On `actix-rt` And `actix-service`](#direct-dependency-on-actix-rt-and-actix-service)
- [Server Must Be Polled :warning:](#server-must-be-polled-warning)
- [Guards API](#guards-api)
- [Returning `HttpResponse` synchronously](#returning-httpresponse-synchronously)
- [`#[actix_web::main]` and `#[tokio::main]`](#actix_webmain-and-tokiomain)
- [`web::block`](#webblock)

## MSRV

The MSRV of Actix Web has been raised from 1.42 to 1.54.

## Tokio v1 Ecosystem

Actix Web v4 is now underpinned by `tokio`'s v1 ecosystem.

`cargo` supports having multiple versions of the same crate within the same dependency tree, but `tokio` v1 does not interoperate transparently with its previous versions (v0.2, v0.1). Some of your dependencies might rely on `tokio`, either directly or indirectly—if they are using an older version of `tokio`, check if an update is available.  
The following command can help you to identify these dependencies:

```sh
# Find all crates in your dependency tree that depend on `tokio`
# It also reports the different versions of `tokio` in your dependency tree.
cargo tree -i tokio

# if you depend on multiple versions of tokio, use this command to
# list the dependencies relying on a specific version of tokio:
cargo tree -i tokio:0.2.25
```

## Module Structure

Lots of modules have been re-organized in this release. If a compile error refers to "item XYZ not found in module..." or "module XYZ not found", check the [documentation on docs.rs](https://docs.rs/actix-web) to search for items' new locations.

## `NormalizePath` Middleware :warning:

The default `NormalizePath` behavior now strips trailing slashes by default. This was the _documented_ behaviour in Actix Web v3, but the _actual_ behaviour differed. The discrepancy has now been resolved.

As a consequence of this change, routes defined with trailing slashes will become inaccessible when using `NormalizePath::default()`. Calling `NormalizePath::default()` will log a warning. We suggest to use `new` or `trim`.

```diff
- #[get("/test/")]
+ #[get("/test")]
  async fn handler() {

  App::new()
-   .wrap(NormalizePath::default())
+   .wrap(NormalizePath::trim())
```

Alternatively, explicitly require trailing slashes: `NormalizePath::new(TrailingSlash::Always)`.

## Server Settings :warning:

Until Actix Web v4, the underlying `actix-server` crate used the number of available **logical** cores as the default number of worker threads. The new default is the number of [physical CPU cores available](https://github.com/actix/actix-net/commit/3a3d654c). For more information about this change, refer to [this analysis](https://github.com/actix/actix-web/issues/957).

If you notice performance regressions, please open a new issue detailing your observations.

## `FromRequest` Trait

The associated type `Config` of `FromRequest` was removed. If you have custom extractors, you can just remove this implementation and refer to config types directly, if required.

```diff
  impl FromRequest for MyExtractor {
-   type Config = ();
  }
```

Consequently, the `FromRequest::configure` method was also removed. Config for extractors is still provided using `App::app_data` but should now be constructed in a standalone way.

## Compression Feature Flags

The `compress` feature flag has been split into more granular feature flags, one for each supported algorithm (brotli, gzip, zstd). By default, all compression algorithms are enabled. If you want to select specific compression codecs, the new flags are:

- `compress-brotli`
- `compress-gzip`
- `compress-zstd`

## `web::Path`

The inner field for `web::Path` is now private. It was causing ambiguity when trying to use tuple indexing due to its `Deref` implementation.

```diff
- async fn handler(web::Path((foo, bar)): web::Path<(String, String)>) {
+ async fn handler(params: web::Path<(String, String)>) {
+   let (foo, bar) = params.into_inner();
```

An alternative [path param type with public field but no `Deref` impl is available in `actix-web-lab`](https://docs.rs/actix-web-lab/0.12.0/actix_web_lab/extract/struct.Path.html).

## Rustls Crate Upgrade

Actix Web now depends on version 0.20 of `rustls`. As a result, the server config builder has changed. [See the updated example project.](https://github.com/actix/examples/tree/master/https-tls/rustls/)

## Removed `awc` Client Re-export

Actix Web's sister crate `awc` is no longer re-exported through the `client` module. This allows `awc` to have its own release cadence—its breaking changes are no longer blocked by Actix Web's (more conservative) release schedule.

```diff
- use actix_web::client::Client;
+ use awc::Client;
```

## Integration Testing Utils Moved To `actix-test`

`TestServer` has been moved to its own crate, [`actix-test`](https://docs.rs/actix-test).

```diff
- use use actix_web::test::start;
+ use use actix_test::start;
```

`TestServer` previously lived in `actix_web::test`, but it depends on `awc` which is no longer part of Actix Web's public API (see above).

## Header APIs

Header related APIs have been standardized across all `actix-*` crates. The terminology now better matches the underlying `HeaderMap` naming conventions.

In short, "insert" always indicates that any existing headers with the same name are overridden, while "append" is used for adding with no removal (e.g. multi-valued headers).

For request and response builder APIs, the new methods provide a unified interface for adding key-value pairs _and_ typed headers, which can often be more expressive.

```diff
- .set_header("Api-Key", "1234")
+ .insert_header(("Api-Key", "1234"))

- .header("Api-Key", "1234")
+ .append_header(("Api-Key", "1234"))

- .set(ContentType::json())
+ .insert_header(ContentType::json())
```

We chose to deprecate most of the old methods instead of removing them immediately—the warning notes will guide you on how to update.

## Response Body Types

There have been a lot of changes to response body types. They are now more expressive and their purpose should be more intuitive.

We have boosted the quality and completeness of the documentation for all items in the [`body` module](https://docs.rs/actix-web/4/actix_web/body).

### `ResponseBody`

`ResponseBody` is gone. Its purpose was confusing and has been replaced by better components.

### `Body`

`Body` is also gone. In combination with `ResponseBody`, the API it provided was sub-optimal and did not encourage expressive types. Here are the equivalents in the new system (check docs):

- `Body::None` => `body::None::new()`
- `Body::Empty` => `()` / `web::Bytes::new()`
- `Body::Bytes` => `web::Bytes::from(...)`
- `Body::Message` => `.boxed()` / `BoxBody`

### `BoxBody`

`BoxBody` is a new type-erased body type.

It can be useful when writing handlers, responders, and middleware when you want to trade a (very) small amount of performance for a simpler type.

Creating a boxed body is done most efficiently by calling [`.boxed()`](https://docs.rs/actix-web/4/actix_web/body/trait.MessageBody.html#method.boxed) on a `MessageBody` type.

### `EitherBody`

`EitherBody` is a new "either" type that implements `MessageBody`

It is particularly useful in middleware that can bail early, returning their own response plus body type. By default the "right" variant is `BoxBody` (i.e., `EitherBody<B>` === `EitherBody<B, BoxBody>`) but it can be anything that implements `MessageBody`.

For example, it will be common among middleware which value performance of the hot path to use:

```rust
type Response = Result<ServiceResponse<EitherBody<B>>, Error>
```

This can be read (ignoring the `Result`) as "resolves with a `ServiceResponse` that is either the inner service's `B` body type or a boxed body type from elsewhere, likely constructed within the middleware itself". Of course, if your middleware contains only simple string other/error responses, it's possible to use them without boxes at the cost of a less simple implementation:

```rust
type Response = Result<ServiceResponse<EitherBody<B, String>>, Error>
```

### Error Handlers

`ErrorHandlers` is a commonly used middleware that has changed in design slightly due to the other body type changes.

In particular, an implicit `EitherBody` is used in the `ErrorHandlerResponse<B>` type. An `ErrorHandlerResponse<B>` now expects a `ServiceResponse<EitherBody<B>>` to be returned within response variants. The following is a migration for an error handler that **only modifies** the response argument (left body).

```diff
  fn add_error_header<B>(mut res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>, Error> {
      res.response_mut().headers_mut().insert(
          header::CONTENT_TYPE,
          header::HeaderValue::from_static("Error"),
      );
-     Ok(ErrorHandlerResponse::Response(res))
+     Ok(ErrorHandlerResponse::Response(res.map_into_left_body()))
  }
```

The following is a migration for an error handler that creates a new response instead (right body).

```diff
  fn error_handler<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>, Error> {
-     let req = res.request().clone();
+     let (req, _res) = res.into_parts();

      let res = actix_files::NamedFile::open("./templates/404.html")?
          .set_status_code(StatusCode::NOT_FOUND)
-         .into_response(&req)?
-         .into_body();
+         .into_response(&req);

-     let res = ServiceResponse::new(req, res);
+     let res = ServiceResponse::new(req, res).map_into_right_body();
      Ok(ErrorHandlerResponse::Response(res))
  }
```

## Middleware Trait APIs

The underlying traits that are used for creating middleware, `Service`, `ServiceFactory`, and `Transform`, have changed in design.

- The associated `Request` type has moved to the type parameter position in order to allow multiple request implementations in other areas of the service stack.
- The `self` arguments in `Service` have changed from exclusive (mutable) borrows to shared (immutable) borrows. Since most service layers, such as middleware, do not host mutable state, it reduces the runtime overhead in places where a `RefCell` used to be required for wrapping an inner service.
- We've also introduced some macros that reduce boilerplate when implementing `poll_ready`.
- Further to the guidance on [response body types](#response-body-types), any use of the old methods on `ServiceResponse` designed to match up body types (e.g., the old `into_body` method), should be replaced with an explicit response body type utilizing `EitherBody<B>`.

A typical migration would look like this:

```diff
  use std::{
-     cell::RefCell,
      future::Future,
      pin::Pin,
      rc::Rc,
-     task::{Context, Poll},
  };

  use actix_web::{
      dev::{Service, ServiceRequest, ServiceResponse, Transform},
      Error,
  };
  use futures_util::future::{ok, LocalBoxFuture, Ready};

  pub struct SayHi;

- impl<S, B> Transform<S> for SayHi
+ impl<S, B> Transform<S, ServiceRequest> for SayHi
  where
-     S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
+     S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
      S::Future: 'static,
      B: 'static,
  {
-     type Request = ServiceRequest;
      type Response = ServiceResponse<B>;
      type Error = Error;
      type InitError = ();
      type Transform = SayHiMiddleware<S>;
      type Future = Ready<Result<Self::Transform, Self::InitError>>;

      fn new_transform(&self, service: S) -> Self::Future {
          ok(SayHiMiddleware {
-             service: Rc::new(RefCell::new(service)),
+             service: Rc::new(service),
          })
      }
  }

  pub struct SayHiMiddleware<S> {
-     service: Rc<RefCell<S>>,
+     service: Rc<S>,
  }

- impl<S, B> Service for SayHiMiddleware<S>
+ impl<S, B> Service<ServiceRequest> for SayHiMiddleware<S>
  where
-     S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
+     S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
      S::Future: 'static,
      B: 'static,
  {
-     type Request = ServiceRequest;
      type Response = ServiceResponse<B>;
      type Error = Error;
      type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

-     fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
-         self.service.poll_ready(cx)
-     }
+     actix_web::dev::forward_ready!(service);

-     fn call(&mut self, req: ServiceRequest) -> Self::Future {
+     fn call(&self, req: ServiceRequest) -> Self::Future {
          println!("Hi from start. You requested: {}", req.path());

          let fut = self.service.call(req);

          Box::pin(async move {
              let res = fut.await?;

              println!("Hi from response");
              Ok(res)
          })
      }
  }
```

This new design is forward-looking and should ease transition to traits that support the upcoming Generic Associated Type (GAT) feature in Rust while also trimming down the boilerplate required to implement middleware.

We understand that creating middleware is still a pain point for Actix Web and we hope to provide [an even more ergonomic solution](https://docs.rs/actix-web-lab/0.11.0/actix_web_lab/middleware/fn.from_fn.html) in a v4.x release.

## `Responder` Trait

The `Responder` trait's interface has changed. Errors should be handled and converted to responses within the `respond_to` method. It's also no longer async so the associated `type Future` has been removed; there was no compelling use case found for it. These changes simplify the interface and implementation a lot.

Now that more emphasis is placed on expressive body types, as explained in the [body types migration section](#response-body-types), this trait has introduced an associated `type Body`. The simplest migration will be to use `BoxBody` + `.map_into_boxed_body()` but if there is a more expressive type for your responder then try to use that instead.

```diff
  impl Responder for &'static str {
-     type Error = Error;
-     type Future = Ready<Result<HttpResponse, Error>>;
+     type Body = &'static str;

-     fn respond_to(self, req: &HttpRequest) -> Self::Future {
+     fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
          let res = HttpResponse::build(StatusCode::OK)
              .content_type("text/plain; charset=utf-8")
              .body(self);

-         ok(res)
+         res
      }
  }
```

## `App::data` Deprecation :warning:

The `App::data` method is deprecated. Replace instances of this with `App::app_data`. Exposing both methods led to lots of confusion when trying to extract the data in handlers. Now, when using the `Data` wrapper, the type you put in to `app_data` is the same type you extract in handler arguments.

You may need to review the [guidance on shared mutable state](https://docs.rs/actix-web/4/actix_web/struct.App.html#shared-mutable-state) in order to migrate this correctly.

```diff
  use actix_web::web::Data;

  #[get("/")]
  async fn handler(my_state: Data<MyState>) -> { todo!() }

  HttpServer::new(|| {
-     App::new()
-         .data(MyState::default())
-         .service(handler)

+     let my_state: Data<MyState> = Data::new(MyState::default());
+
+     App::new()
+         .app_data(my_state)
+         .service(handler)
  })
```

## Direct Dependency On `actix-rt` And `actix-service`

Improvements to module management and re-exports have resulted in not needing direct dependencies on these underlying crates for the vast majority of cases. In particular:

- all traits necessary for creating middlewares are now re-exported through the `dev` modules;
- `#[actix_web::test]` now exists for async test definitions.

Relying on these re-exports will ease the transition to future versions of Actix Web.

```diff
- use actix_service::{Service, Transform};
+ use actix_web::dev::{Service, Transform};
```

```diff
- #[actix_rt::test]
+ #[actix_web::test]
  async fn test_thing() {
```

## Server Must Be Polled :warning:

In order to _start_ serving requests, the `Server` object returned from `run` **must** be `poll`ed, `await`ed, or `spawn`ed. This was done to prevent unexpected behavior and ensure that things like signal handlers are able to function correctly when enabled.

For example, in this contrived example where the server is started and then the main thread is sent to sleep, the server will no longer be able to serve requests with v4.0:

```rust
#[actix_web::main]
async fn main() {
    HttpServer::new(|| App::new().default_service(web::to(HttpResponse::Conflict)))
        .bind(("127.0.0.1", 8080))
        .unwrap()
        .run();

    thread::sleep(Duration::from_secs(1000));
}
```

## Guards API

Implementors of routing guards will need to use the modified interface of the `Guard` trait. The API is more flexible than before. See [guard module docs](https://docs.rs/actix-web/4/actix_web/guard/struct.GuardContext.html) for more details.

```diff
  struct MethodGuard(HttpMethod);

  impl Guard for MethodGuard {
-     fn check(&self, request: &RequestHead) -> bool {
+     fn check(&self, ctx: &GuardContext<'_>) -> bool {
-         request.method == self.0
+         ctx.head().method == self.0
      }
  }
```

## Returning `HttpResponse` synchronously

The implementation of `Future` for `HttpResponse` was removed because it was largely useless for all but the simplest handlers like `web::to(|| HttpResponse::Ok().finish())`. It also caused false positives on the `async_yields_async` clippy lint in reasonable scenarios. The compiler errors will looks something like:

```
web::to(|| HttpResponse::Ok().finish())
^^^^^^^ the trait `Handler<_>` is not implemented for `[closure@...]`
```

This form should be replaced with explicit async functions and closures:

```diff
- fn handler() -> HttpResponse {
+ async fn handler() -> HttpResponse {
      HttpResponse::Ok().finish()
  }
```

```diff
- web::to(|| HttpResponse::Ok().finish())
+ web::to(|| async { HttpResponse::Ok().finish() })
```

Or, for these extremely simple cases, utilise an `HttpResponseBuilder`:

```diff
- web::to(|| HttpResponse::Ok().finish())
+ web::to(HttpResponse::Ok)
```

## `#[actix_web::main]` and `#[tokio::main]`

Actix Web now works seamlessly with the primary way of starting a multi-threaded Tokio runtime, `#[tokio::main]`. Therefore, it is no longer necessary to spawn a thread when you need to run something alongside Actix Web that uses Tokio's multi-threaded mode; you can simply await the server within this context or, if preferred, use `tokio::spawn` just like any other async task.

For now, `actix` actor support (and therefore WebSocket support via `actix-web-actors`) still requires `#[actix_web::main]` so that a `System` context is created. Designs are being created for an alternative WebSocket interface that does not require actors that should land sometime in the v4.x cycle.

## `web::block`

The `web::block` helper has changed return type from roughly `async fn(fn() -> Result<T, E>) Result<T, BlockingError<E>>` to `async fn(fn() -> T) Result<T, BlockingError>`. That's to say that the blocking function can now return things that are not `Result`s and it does not wrap error types anymore. If you still need to return `Result`s then you'll likely want to use double `?` after the `.await`.

```diff
- let n: u32 = web::block(|| Ok(123)).await?;
+ let n: u32 = web::block(|| 123).await?;

- let n: u32 = web::block(|| Ok(123)).await?;
+ let n: u32 = web::block(|| Ok(123)).await??;
```

## `HttpResponse` as a `ResponseError`

The implementation of `ResponseError` for `HttpResponse` has been removed.

It was common in v3 to use `HttpResponse` as an error type in fallible handlers. The problem is that `HttpResponse` contains no knowledge or reference to the source error. Being able to guarantee that an "error" response actually contains an error reference makes middleware and other parts of Actix Web more effective.

The error response builders in the `error` module were available in v3 but are now the best method for simple error responses without requiring you to implement the trait on your own custom error types. These builders can receive simple strings and third party errors that can not implement the `ResponseError` trait.

A few common patterns are affected by this change:

```diff
- Err(HttpResponse::InternalServerError().finish())
+ Err(error::ErrorInternalServerError("reason"))

- Err(HttpResponse::InternalServerError().body(third_party_error.to_string()))
+ Err(error::ErrorInternalServerError(err))

- .map_err(|err| HttpResponse::InternalServerError().finish())?
+ .map_err(error::ErrorInternalServerError)?
```
