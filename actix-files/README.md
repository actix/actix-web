# `actix-files`

<!-- prettier-ignore-start -->

[![crates.io](https://img.shields.io/crates/v/actix-files?label=latest)](https://crates.io/crates/actix-files)
[![Documentation](https://docs.rs/actix-files/badge.svg?version=0.6.6)](https://docs.rs/actix-files/0.6.6)
![Version](https://img.shields.io/badge/rustc-1.72+-ab6000.svg)
![License](https://img.shields.io/crates/l/actix-files.svg)
<br />
[![dependency status](https://deps.rs/crate/actix-files/0.6.6/status.svg)](https://deps.rs/crate/actix-files/0.6.6)
[![Download](https://img.shields.io/crates/d/actix-files.svg)](https://crates.io/crates/actix-files)
[![Chat on Discord](https://img.shields.io/discord/771444961383153695?label=chat&logo=discord)](https://discord.gg/NWpN5mmg3x)

<!-- prettier-ignore-end -->

<!-- cargo-rdme start -->

Static file serving for Actix Web.

Provides a non-blocking service for serving static files from disk.

## Examples

```rust
use actix_web::App;
use actix_files::Files;

let app = App::new()
    .service(Files::new("/static", ".").prefer_utf8(true));
```

<!-- cargo-rdme end -->
