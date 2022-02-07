# Migrating to 4.0.0

It is assumed that migration is happening _from_ v3.x. If migration from older version of Actix Web, see the other historical migration notes in this folder.

This is not an exhaustive list of changes. Smaller or less impactful code changes are outlined, with links to the PRs that introduced them, are shown in [CHANGES.md](./CHANGES.md). If you think any of the changes not mentioned here deserve to be, submit an issue or PR.

Headings marked with :warning: are **breaking behavioral changes** and will probably not surface as compile-time errors. Automated tests _might_ detect their effects on your app.

## Table of Contents:

- [MSRV](#msrv)
- [Server Settings](#server-settings)
- [Module Structure](#module-structure)
- [`NormalizePath` Middleware :warning:](#normalizepath-middleware-warning)
- [`FromRequest` Trait](#fromrequest-trait)
- [Compression Feature Flags](#compression-feature-flags)
- [`web::Path`](#webpath)
- [Rustls](#rustls-crate-upgrade)

## MSRV

The MSRV of Actix Web has been raised from 1.42 to 1.54.

## Server Settings

Until actix-web v4, actix-server used the total number of available logical cores as the default number of worker threads.  The new default number of worker threads for actix-server is the number of [physical CPU cores available](https://github.com/actix/actix-net/commit/3a3d654cea5e55b169f6fd05693b765799733b1b#diff-96893e8cb2125e6eefc96105a8462c4fd834943ef5129ffbead1a114133ebb78).  For more information about this change, refer to [this analysis](https://github.com/actix/actix-web/issues/957).


## Module Structure

Lots of modules has been organized in this release. If a compile error refers to "item XYZ not found in module..." or "module XYZ not found", refer to the [documentation on docs.rs](https://docs.rs/actix-web) to to search for items' new locations.

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

## `FromRequest` Trait

The associated type `Config` of `FromRequest` was removed. If you have custom extractors, you can just remove this implementation and refer to config types directly, if required.

```diff
  impl FromRequest for MyExtractor {
-   type Config = ();
  }
```

Consequently, the `FromRequest::configure` method was also removed. Config for extractors is still provided using `App::app_data` but should now be constructed in a standalone way.

## Compression Feature Flags

Feature flag `compress` has been split into its supported algorithm (brotli, gzip, zstd). By default, all compression algorithms are enabled. The new flags are:

- `compress-brotli`
- `compress-gzip`
- `compress-zstd`

If you have set in your `Cargo.toml` dedicated `actix-web` features and you still want to have compression enabled.

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

## Integration Testing Utils Moved to `actix-test`

Actix Web's `test` module used to contain `TestServer`. Since this required the `awc` client and it was removed as a re-export (see above), it was moved to its own crate [`actix-test`](https://docs.rs/actix-test).

```diff
- use use actix_web::test::start;
+ use use actix_test::start;
```

## Header APIs

TODO

## Body Types / Removal of Body+ResponseBody types / Addition of EitherBody

TODO

In particular, folks seem to be struggling with the `ErrorHandlers` middleware because of this change and the obscured nature of `EitherBody` within its types.

## Middleware Trait APIs

TODO

TODO: Also write the Middleware author's guide.

## `Responder` Trait

TODO

## `App::data` deprecation

TODO

## It's probably not necessary to import `actix-rt` or `actix-service` any more

TODO

## Server must be awaited in order to run :warning:

TODO

## Guards API

TODO

## HttpResponse no longer implements Future

TODO
