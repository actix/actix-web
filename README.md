<div align="center">
 <p><h1>Actix web</h1> </p>
  <p><strong>Actix web is a small, pragmatic, and extremely fast rust web framework</strong> </p>
  <p>

[![Build Status](https://travis-ci.org/actix/actix-web.svg?branch=master)](https://travis-ci.org/actix/actix-web) 
[![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web) 
[![crates.io](https://meritbadge.herokuapp.com/actix-web)](https://crates.io/crates/actix-web)
[![Join the chat at https://gitter.im/actix/actix](https://badges.gitter.im/actix/actix.svg)](https://gitter.im/actix/actix?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)
[![Documentation](https://docs.rs/actix-web/badge.svg)](https://docs.rs/actix-web)
[![Download](https://img.shields.io/crates/d/actix-web.svg)](https://crates.io/crates/actix-web)
[![Version](https://img.shields.io/badge/rustc-1.39+-lightgray.svg)](https://blog.rust-lang.org/2019/11/07/Rust-1.39.0.html)
![License](https://img.shields.io/crates/l/actix-web.svg)

  </p>

  <h3>
    <a href="https://actix.rs">Website</a>
    <span> | </span>
    <a href="https://gitter.im/actix/actix">Chat</a>
    <span> | </span>
    <a href="https://github.com/actix/examples">Examples</a>
  </h3>
</div>
<br>

Actix web is a simple, pragmatic and extremely fast web framework for Rust.

* Supported *HTTP/1.x* and *HTTP/2.0* protocols
* Streaming and pipelining
* Keep-alive and slow requests handling
* Client/server [WebSockets](https://actix.rs/docs/websockets/) support
* Transparent content compression/decompression (br, gzip, deflate)
* Configurable [request routing](https://actix.rs/docs/url-dispatch/)
* Multipart streams
* Static assets
* SSL support with OpenSSL or Rustls
* Middlewares ([Logger, Session, CORS, etc](https://actix.rs/docs/middleware/))
* Includes an asynchronous [HTTP client](https://actix.rs/actix-web/actix_web/client/index.html)
* Supports [Actix actor framework](https://github.com/actix/actix)

## Example

Dependencies:

```toml
[dependencies]
actix-web = "2"
actix-rt = "1"
```

Code:

```rust
use actix_web::{get, web, App, HttpServer, Responder};

#[get("/{id}/{name}/index.html")]
async fn index(info: web::Path<(u32, String)>) -> impl Responder {
    format!("Hello {}! id:{}", info.1, info.0)
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().service(index))
        .bind("127.0.0.1:8080")?
        .run()
        .await
}
```

### More examples

* [Basics](https://github.com/actix/examples/tree/master/basics/)
* [Stateful](https://github.com/actix/examples/tree/master/state/)
* [Multipart streams](https://github.com/actix/examples/tree/master/multipart/)
* [Simple websocket](https://github.com/actix/examples/tree/master/websocket/)
* [Tera](https://github.com/actix/examples/tree/master/template_tera/) /
* [Askama](https://github.com/actix/examples/tree/master/template_askama/) templates
* [Diesel integration](https://github.com/actix/examples/tree/master/diesel/)
* [r2d2](https://github.com/actix/examples/tree/master/r2d2/)
* [OpenSSL](https://github.com/actix/examples/tree/master/openssl/)
* [Rustls](https://github.com/actix/examples/tree/master/rustls/)
* [Tcp/Websocket chat](https://github.com/actix/examples/tree/master/websocket-chat/)
* [Json](https://github.com/actix/examples/tree/master/json/)

You may consider checking out
[this directory](https://github.com/actix/examples/tree/master/) for more examples.

## Benchmarks

* [TechEmpower Framework Benchmark](https://www.techempower.com/benchmarks/#section=data-r18)

## License

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))
* MIT license ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))

at your option.

## Code of Conduct

Contribution to the actix-web crate is organized under the terms of the
Contributor Covenant, the maintainer of actix-web, @fafhrd91, promises to
intervene to uphold that code of conduct.
