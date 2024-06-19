<div align="center">
  <h1>Actix Web</h1>
  <p>
    <strong>Actix Web is a powerful, pragmatic, and extremely fast web framework for Rust</strong>
  </p>
  <p>

<!-- prettier-ignore-start -->

[![crates.io](https://img.shields.io/crates/v/actix-web?label=latest)](https://crates.io/crates/actix-web)
[![Documentation](https://docs.rs/actix-web/badge.svg?version=4.8.0)](https://docs.rs/actix-web/4.8.0)
![MSRV](https://img.shields.io/badge/rustc-1.72+-ab6000.svg)
![MIT or Apache 2.0 licensed](https://img.shields.io/crates/l/actix-web.svg)
[![Dependency Status](https://deps.rs/crate/actix-web/4.8.0/status.svg)](https://deps.rs/crate/actix-web/4.8.0)
<br />
[![CI](https://github.com/actix/actix-web/actions/workflows/ci.yml/badge.svg)](https://github.com/actix/actix-web/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web)
![downloads](https://img.shields.io/crates/d/actix-web.svg)
[![Chat on Discord](https://img.shields.io/discord/771444961383153695?label=chat&logo=discord)](https://discord.gg/NWpN5mmg3x)

<!-- prettier-ignore-end -->

  </p>
</div>

## Features

- Supports _HTTP/1.x_ and _HTTP/2_
- Streaming and pipelining
- Powerful [request routing](https://actix.rs/docs/url-dispatch/) with optional macros
- Full [Tokio](https://tokio.rs) compatibility
- Keep-alive and slow requests handling
- Client/server [WebSockets](https://actix.rs/docs/websockets/) support
- Transparent content compression/decompression (br, gzip, deflate, zstd)
- Multipart streams
- Static assets
- SSL support using OpenSSL or Rustls
- Middlewares ([Logger, Session, CORS, etc](https://actix.rs/docs/middleware/))
- Integrates with the [`awc` HTTP client](https://docs.rs/awc/)
- Runs on stable Rust 1.72+

## Documentation

- [Website & User Guide](https://actix.rs)
- [Examples Repository](https://github.com/actix/examples)
- [API Documentation](https://docs.rs/actix-web)
- [API Documentation (master branch)](https://actix.rs/actix-web/actix_web)

## Example

Dependencies:

```toml
[dependencies]
actix-web = "4"
```

Code:

```rust
use actix_web::{get, web, App, HttpServer, Responder};

#[get("/hello/{name}")]
async fn greet(name: web::Path<String>) -> impl Responder {
    format!("Hello {name}!")
}

#[actix_web::main] // or #[tokio::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new().service(greet)
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
```

### More Examples

- [Hello World](https://github.com/actix/examples/tree/master/basics/hello-world)
- [Basic Setup](https://github.com/actix/examples/tree/master/basics/basics)
- [Application State](https://github.com/actix/examples/tree/master/basics/state)
- [JSON Handling](https://github.com/actix/examples/tree/master/json/json)
- [Multipart Streams](https://github.com/actix/examples/tree/master/forms/multipart)
- [MongoDB Integration](https://github.com/actix/examples/tree/master/databases/mongodb)
- [Diesel Integration](https://github.com/actix/examples/tree/master/databases/diesel)
- [SQLite Integration](https://github.com/actix/examples/tree/master/databases/sqlite)
- [Postgres Integration](https://github.com/actix/examples/tree/master/databases/postgres)
- [Tera Templates](https://github.com/actix/examples/tree/master/templating/tera)
- [Askama Templates](https://github.com/actix/examples/tree/master/templating/askama)
- [HTTPS using Rustls](https://github.com/actix/examples/tree/master/https-tls/rustls)
- [HTTPS using OpenSSL](https://github.com/actix/examples/tree/master/https-tls/openssl)
- [Simple WebSocket](https://github.com/actix/examples/tree/master/websockets)
- [WebSocket Chat](https://github.com/actix/examples/tree/master/websockets/chat)

You may consider checking out [this directory](https://github.com/actix/examples/tree/master) for more examples.

## Benchmarks

One of the fastest web frameworks available according to the [TechEmpower Framework Benchmark](https://www.techempower.com/benchmarks/#section=data-r21&test=composite).

## License

This project is licensed under either of the following licenses, at your option:

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0])
- MIT license ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT])

## Code of Conduct

Contribution to the `actix/actix-web` repo is organized under the terms of the Contributor Covenant. The Actix team promises to intervene to uphold that code of conduct.
