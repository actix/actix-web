# Migrating to 4.0.0

> It is assumed that migration is happening _from_ v3.x. If migration from older version of Actix Web, see [the historical migration notes](./MIGRATION-3.0.md).

## Rustls Upgrade

Required version of Rustls dependency was bumped to the latest version 0.20. As a result, the new server config builder has changed. [See the updated example project &rarr;.](https://github.com/actix/examples/tree/HEAD/security/rustls/)

### `NormalizePath` middleware

The default `NormalizePath` behavior now strips trailing slashes by default. This was previously documented to be the case in v3 but the behavior now matches. The effect is that routes defined with trailing slashes will become inaccessible when using `NormalizePath::default()`. As such, calling `NormalizePath::default()` will log a warning. It is advised that the `new` or `trim` methods be used instead.

#### Migration Diff

```diff
- #[get("/test/")]`
+ #[get("/test")]`

- .wrap(NormalizePath::default())`
+ .wrap(NormalizePath::trim())`
```

Alternatively, explicitly require trailing slashes: `NormalizePath::new(TrailingSlash::Always)`.

### `FromRequest` trait

The associated type `Config` of `FromRequest` was removed.

### Compression Feature Flags

Feature flag `compress` has been split into its supported algorithm (brotli, gzip, zstd). By default, all compression algorithms are enabled. The new flags are:

- `compress-brotli`
- `compress-gzip`
- `compress-zstd`

If you have set in your `Cargo.toml` dedicated `actix-web` features and you still want to have compression enabled.
