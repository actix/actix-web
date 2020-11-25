<div align="center">
  <h1>Actix web</h1>
  <p>
    <strong>Actix web is a powerful, pragmatic, and extremely fast web framework for Rust</strong>
  </p>
  <p>

[![crates.io](https://img.shields.io/crates/v/actix-web?label=latest)](https://crates.io/crates/actix-web)
[![Documentation](https://docs.rs/actix-web/badge.svg?version=3.3.0)](https://docs.rs/actix-web/3.3.0)
[![Version](https://img.shields.io/badge/rustc-1.42+-ab6000.svg)](https://blog.rust-lang.org/2020/03/12/Rust-1.42.html)
![License](https://img.shields.io/crates/l/actix-web.svg)
[![Dependency Status](https://deps.rs/crate/actix-web/3.3.0/status.svg)](https://deps.rs/crate/actix-web/3.3.0)
<br />
[![Build Status](https://travis-ci.org/actix/actix-web.svg?branch=master)](https://travis-ci.org/actix/actix-web) 
[![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web) 
[![Download](https://img.shields.io/crates/d/actix-web.svg)](https://crates.io/crates/actix-web)
[![Join the chat at https://gitter.im/actix/actix](https://badges.gitter.im/actix/actix.svg)](https://gitter.im/actix/actix?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)
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
* Includes an async [HTTP client](https://actix.rs/actix-web/actix_web/client/index.html)
* Supports [Actix actor framework](https://github.com/actix/actix)
* Runs on stable Rust 1.42+

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

* [Basic Setup](https://github.com/actix/examples/tree/master/basics/)
* [Application State](https://github.com/actix/examples/tree/master/state/)
* [JSON Handling](https://github.com/actix/examples/tree/master/json/)
* [Multipart Streams](https://github.com/actix/examples/tree/master/multipart/)
* [Diesel Integration](https://github.com/actix/examples/tree/master/diesel/)
* [r2d2 Integration](https://github.com/actix/examples/tree/master/r2d2/)
* [Simple WebSocket](https://github.com/actix/examples/tree/master/websocket/)
* [Tera Templates](https://github.com/actix/examples/tree/master/template_tera/)
* [Askama Templates](https://github.com/actix/examples/tree/master/template_askama/)
* [HTTPS using Rustls](https://github.com/actix/examples/tree/master/rustls/)
* [HTTPS using OpenSSL](https://github.com/actix/examples/tree/master/openssl/)
* [WebSocket Chat](https://github.com/actix/examples/tree/master/websocket-chat/)

You may consider checking out
[this directory](https://github.com/actix/examples/tree/master/) for more examples.

## Benchmarks

One of the fastest web frameworks available according to the
[TechEmpower Framework Benchmark](https://www.techempower.com/benchmarks/#section=data-r19).

## License

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
  [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))
* MIT license ([LICENSE-MIT](LICENSE-MIT) or
  [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))

at your option.

## Code of Conduct

Contribution to the actix-web crate is organized under the terms of the Contributor Covenant, the
maintainers of Actix web, promises to intervene to uphold that code of conduct.
