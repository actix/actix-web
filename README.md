<div align="center">
  <h1>Actix Web</h1>
  <p>
    <strong>Actix Web is a powerful, pragmatic, and extremely fast web framework for Rust</strong>
  </p>
  <p>

[![crates.io](https://img.shields.io/crates/v/actix-web?label=latest)](https://crates.io/crates/actix-web)
[![Documentation](https://docs.rs/actix-web/badge.svg?version=4.0.0-beta.5)](https://docs.rs/actix-web/4.0.0-beta.5)
[![Version](https://img.shields.io/badge/rustc-1.46+-ab6000.svg)](https://blog.rust-lang.org/2020/03/12/Rust-1.46.html)
![MIT or Apache 2.0 licensed](https://img.shields.io/crates/l/actix-web.svg)
[![Dependency Status](https://deps.rs/crate/actix-web/4.0.0-beta.5/status.svg)](https://deps.rs/crate/actix-web/4.0.0-beta.5)
<br />
[![build status](https://github.com/actix/actix-web/workflows/CI%20%28Linux%29/badge.svg?branch=master&event=push)](https://github.com/actix/actix-web/actions)
[![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web) 
![downloads](https://img.shields.io/crates/d/actix-web.svg)
[![Chat on Discord](https://img.shields.io/discord/771444961383153695?label=chat&logo=discord)](https://discord.gg/NWpN5mmg3x)

  </p>
</div>

## Features

* Supports *HTTP/1.x* and *HTTP/2*
* Streaming and pipelining
* Keep-alive and slow requests handling
* Client/server [WebSockets](https://actix.rs/docs/websockets/) support
* Transparent content compression/decompression (br, gzip, deflate)
* Powerful [request routing](https://actix.rs/docs/url-dispatch/)
* Multipart streams
* Static assets
* SSL support using OpenSSL or Rustls
* Middlewares ([Logger, Session, CORS, etc](https://actix.rs/docs/middleware/))
* Includes an async [HTTP client](https://docs.rs/actix-web/latest/actix_web/client/index.html)
* Runs on stable Rust 1.46+

## Documentation

* [Website & User Guide](https://actix.rs)
* [Examples Repository](https://github.com/actix/examples)
* [API Documentation](https://docs.rs/actix-web)
* [API Documentation (master branch)](https://actix.rs/actix-web/actix_web)

## Example

Dependencies:

```toml
[dependencies]
actix-web = "3"
```

Code:

```rust
use actix_web::{get, web, App, HttpServer, Responder};

#[get("/{id}/{name}/index.html")]
async fn index(web::Path((id, name)): web::Path<(u32, String)>) -> impl Responder {
    format!("Hello {}! id:{}", name, id)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().service(index))
        .bind("127.0.0.1:8080")?
        .run()
        .await
}
```

### More examples

* [Basic Setup](https://github.com/actix/examples/tree/master/basics/basics/)
* [Application State](https://github.com/actix/examples/tree/master/basics/state/)
* [JSON Handling](https://github.com/actix/examples/tree/master/json/json/)
* [Multipart Streams](https://github.com/actix/examples/tree/master/forms/multipart/)
* [Diesel Integration](https://github.com/actix/examples/tree/master/database_interactions/diesel/)
* [r2d2 Integration](https://github.com/actix/examples/tree/master/database_interactions/r2d2/)
* [Simple WebSocket](https://github.com/actix/examples/tree/master/websockets/websocket/)
* [Tera Templates](https://github.com/actix/examples/tree/master/template_engines/tera/)
* [Askama Templates](https://github.com/actix/examples/tree/master/template_engines/askama/)
* [HTTPS using Rustls](https://github.com/actix/examples/tree/master/security/rustls/)
* [HTTPS using OpenSSL](https://github.com/actix/examples/tree/master/security/openssl/)
* [WebSocket Chat](https://github.com/actix/examples/tree/master/websockets/chat/)

You may consider checking out
[this directory](https://github.com/actix/examples/tree/master/) for more examples.

## Benchmarks

One of the fastest web frameworks available according to the
[TechEmpower Framework Benchmark](https://www.techempower.com/benchmarks/#section=data-r20&test=composite).

## License

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
  [http://www.apache.org/licenses/LICENSE-2.0])
* MIT license ([LICENSE-MIT](LICENSE-MIT) or
  [http://opensource.org/licenses/MIT])

at your option.

## Code of Conduct

Contribution to the actix-web repo is organized under the terms of the Contributor Covenant.
The Actix team promises to intervene to uphold that code of conduct.
