# Migrating to 4.0.0

It is assumed that migration is happening _from_ v3.x. If migration from older version of Actix Web, see the other historical migration notes in this folder.

This is not an exhaustive list of changes. Smaller or less impactful code changes are outlined, with links to the PRs that introduced them, in [CHANGES.md](./CHANGES.md). If you think any of the changes not mentioned here deserve to be, submit an issue or PR.

Headings marked with :warning: are **breaking behavioral changes** that will probably not surface as compile-time errors though automated tests _might_ detect their effects on your app.

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
- [`#[actix_web::main]` and `#[tokio::main]`](#actixwebmain-and-tokiomain)
- [`web::block`](#webblock)

## MSRV

The MSRV of Actix Web has been raised from 1.42 to 1.54.

## Tokio v1 Ecosystem

Actix Web v4 is now underpinned by the the Tokio v1 ecosystem of crates. If you have dependencies that might utilize Tokio directly, it is worth checking to see if an update is available. The following command will assist in finding such dependencies:

```sh
cargo tree -i tokio

# if multiple tokio versions are depended on, show the older ones with:
cargo tree -i tokio:0.2.25
```

## Module Structure

Lots of modules has been organized in this release. If a compile error refers to "item XYZ not found in module..." or "module XYZ not found", refer to the [documentation on docs.rs](https://docs.rs/actix-web) to search for items' new locations.

## `NormalizePath` Middleware :warning:

The default `NormalizePath` behavior now strips trailing slashes by default. This was previously documented to be the case in v3 but the behavior now matches. The effect is that routes defined with trailing slashes will become inaccessible when using `NormalizePath::default()`. As such, calling `NormalizePath::default()` will log a warning. It is advised that the `new` or `trim` methods be used instead.

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

Feature flag `compress` has been split into its supported algorithm (brotli, gzip, zstd). By default, all compression algorithms are enabled. If you want to select specific compression codecs, the new flags are:

- `compress-brotli`
- `compress-gzip`
- `compress-zstd`

## `web::Path`

The inner field for `web::Path` was made private because It was causing too many issues when used with inner tuple types due to its `Deref` impl.

```diff
- async fn handler(web::Path((foo, bar)): web::Path<(String, String)>) {
+ async fn handler(params: web::Path<(String, String)>) {
+   let (foo, bar) = params.into_inner();
```

## Rustls Crate Upgrade

Required version of `rustls` dependency was bumped to the latest version 0.20. As a result, the new server config builder has changed. [See the updated example project &rarr;.](https://github.com/actix/examples/tree/HEAD/security/rustls/)

## Removed `awc` Client Re-export

Actix Web's sister crate `awc` is no longer re-exported through the `client` module. This allows `awc` its own release cadence and prevents its own breaking changes from being blocked due to a re-export.

```diff
- use actix_web::client::Client;
+ use awc::Client;
```

## Integration Testing Utils Moved To `actix-test`

Actix Web's `test` module used to contain `TestServer`. Since this required the `awc` client and it was removed as a re-export (see above), it was moved to its own crate [`actix-test`](https://docs.rs/actix-test).

```diff
- use use actix_web::test::start;
+ use use actix_test::start;
```

## Header APIs

Header related APIs have been standardized across all `actix-*` crates. The terminology now better matches the underlying `HeaderMap` naming conventions. Most of the the old methods have only been deprecated with notes that will guide how to update.

In short, "insert" always indicates that existing any existing headers with the same name are overridden and "append" indicates adding with no removal.

For request and response builder APIs, the new methods provide a unified interface for adding key-value pairs _and_ typed headers, which can often be more expressive.

```diff
- .set_header("Api-Key", "1234")
+ .insert_header(("Api-Key", "1234"))

- .header("Api-Key", "1234")
+ .append_header(("Api-Key", "1234"))

- .set(ContentType::json())
+ .insert_header(ContentType::json())
```

## Response Body Types

There have been a lot of changes to response body types. The general theme is that they are now more expressive and their purposes are more obvious.

All items in the [`body` module](https://docs.rs/actix-web/4/actix_web/body) have much better documentation now.

### `ResponseBody`

`ResponseBody` is gone. Its purpose was confusing and has been replaced by better components.

### `Body`

`Body` is also gone. In combination with `ResponseBody`, the API it provided was sub-optimal and did not encourage expressive types. Here are the equivalents in the new system (check docs):

- `Body::None` => `body::None::new()`
- `Body::Empty` => `()` / `web::Bytes::new()`
- `Body::Bytes` => `web::Bytes::from(...)`
- `Body::Message` => `.boxed()` / `BoxBody`

### `BoxBody`

`BoxBody` is a new type erased body type. It's used for all error response bodies use this. Creating a boxed body is best done by calling [`.boxed()`](https://docs.rs/actix-web/4/actix_web/body/trait.MessageBody.html#method.boxed) on a `MessageBody` type.

### `EitherBody`

`EitherBody` is a new "either" type that is particularly useful in middleware that can bail early, returning their own response plus body type.

### Error Handlers

TODO In particular, folks seem to be struggling with the `ErrorHandlers` middleware because of this change and the obscured nature of `EitherBody` within its types.

## Middleware Trait APIs

This section builds upon guidance from the [response body types](#response-body-types) section.

TODO

TODO: Also write the Middleware author's guide.

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

The `App::data` method is deprecated. Replace instances of this with `App::app_data`. Exposing both methods was a footgun and lead to lots of confusion when trying to extract the data in handlers. Now, when using the `Data` wrapper, the type you put in to `app_data` is the same type you extract in handler arguments.

You may need to review the [guidance on shared mutable state](https://docs.rs/actix-web/4/actix_web/struct.App.html#shared-mutable-state) in order to migrate this correctly.

```diff
  use actix_web::web::Data;

  #[get("/")]
  async fn handler(my_state: Data<MyState>) -> { todo!() }

  HttpServer::new(|| {
-     App::new()
-         .data(MyState::default())
-         .service(hander)

+     let my_state: Data<MyState> = Data::new(MyState::default());
+
+     App::new()
+         .app_data(my_state)
+         .service(hander)
  })
```

## Direct Dependency On `actix-rt` And `actix-service`

Improvements to module management and re-exports have resulted in not needing direct dependencies on these underlying crates for the vast majority of cases. In particular, all traits necessary for creating middleware are re-exported through the `dev` modules and `#[actix_web::test]` now exists for async test definitions. Relying on the these re-exports will ease transition to future versions of Actix Web.

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

Implementors of routing guards will need to use the modified interface of the `Guard` trait. The API provided is more flexible than before. See [guard module docs](https://docs.rs/actix-web/4/actix_web/guard/struct.GuardContext.html) for more details.

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

Actix Web now works seamlessly with the primary way of starting a multi-threaded Tokio runtime, `#[tokio::main]`. Therefore, it is no longer necessary to spawn a thread when you need to run something alongside Actix Web that uses of Tokio's multi-threaded mode; you can simply await the server within this context or, if preferred, use `tokio::spawn` just like any other async task.

For now, `actix` actor support (and therefore WebSocket support via `actix-web-actors`) still requires `#[actix_web::main]` so that a `System` context is created. Designs are being created for an alternative WebSocket interface that does not require actors that should land sometime in the v4.x cycle.

## `web::block`

The `web::block` helper has changed return type from roughly `async fn(fn() -> Result<T, E>) Result<T, BlockingError<E>>` to `async fn(fn() -> T) Result<T, BlockingError>`. That's to say that the blocking function can now return things that are not `Result`s and it does not wrap error types anymore. If you still need to return `Result`s then you'll likely want to use double `?` after the `.await`.

```diff
- let n: u32 = web::block(|| Ok(123)).await?;
+ let n: u32 = web::block(|| 123).await?;

- let n: u32 = web::block(|| Ok(123)).await?;
+ let n: u32 = web::block(|| Ok(123)).await??;
```
