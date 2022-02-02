# Migrating to 4.0.0

It is assumed that migration is happening _from_ v3.x. If migration from older version of Actix Web, see the other historical migration notes in this folder.

Headings marked with :warning: are **breaking behavioral changes** and will probably not surface as compile-time errors. Automated tests _might_ detect their effects on your app.

## Table of Contents:

- [MSRV](#msrv)
- [Module Structure](#module-structure)
- [`NormalizePath` Middleware :warning:](#normalizepath-middleware-warning)
- [`FromRequest` Trait](#fromrequest-trait)
- [Compression Feature Flags](#compression-feature-flags)
- [`web::Path`](#webpath)
- [Rustls](#rustls-crate-upgrade)

## MSRV

The MSRV of Actix Web has been raised from 1.42 to 1.54.

## Module Structure

Lots of modules has been organized in this release. If a compile error refers to "item XYZ not found in module..." or "module XYZ not found", refer to the [documentation on docs.rs](https://docs.rs/actix-web) to to search for items' new locations.

## `NormalizePath` Middleware :warning:

The default `NormalizePath` behavior now strips trailing slashes by default. This was previously documented to be the case in v3 but the behavior now matches. The effect is that routes defined with trailing slashes will become inaccessible when using `NormalizePath::default()`. As such, calling `NormalizePath::default()` will log a warning. It is advised that the `new` or `trim` methods be used instead.

### Recommended Migration

```diff
- #[get("/test/")]`
+ #[get("/test")]`
  async fn handler() {

  App::new()
-   .wrap(NormalizePath::default())`
+   .wrap(NormalizePath::trim())`
```

Alternatively, explicitly require trailing slashes: `NormalizePath::new(TrailingSlash::Always)`.

## `FromRequest` Trait

The associated type `Config` of `FromRequest` was removed. If you have custom extractors, you can just remove this implementation and refer to config types directly, if required.

### Recommended Migration

```diff
  impl FromRequest for MyExtractor {
-   `type Config = ();`
  }
```

## Compression Feature Flags

Feature flag `compress` has been split into its supported algorithm (brotli, gzip, zstd). By default, all compression algorithms are enabled. The new flags are:

- `compress-brotli`
- `compress-gzip`
- `compress-zstd`

If you have set in your `Cargo.toml` dedicated `actix-web` features and you still want to have compression enabled.

## `web::Path`

The inner field for `web::Path` was made private because It was causing too many issues when used with inner tuple types due to its `Deref` impl.

### Recommended Migration

```diff
- async fn handler(web::Path((foo, bar)): web::Path<(String, String)>) {
+ async fn handler(params: web::Path<(String, String)>) {
+   let (foo, bar) = params.into_inner();
```

## Rustls Crate Upgrade

Required version of `rustls` dependency was bumped to the latest version 0.20. As a result, the new server config builder has changed. [See the updated example project &rarr;.](https://github.com/actix/examples/tree/HEAD/security/rustls/)
