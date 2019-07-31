# Actix web [![Build Status](https://travis-ci.org/actix/actix-web.svg?branch=master)](https://travis-ci.org/actix/actix-web) [![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web) [![crates.io](https://meritbadge.herokuapp.com/actix-web)](https://crates.io/crates/actix-web) [![Join the chat at https://gitter.im/actix/actix](https://badges.gitter.im/actix/actix.svg)](https://gitter.im/actix/actix?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)

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

## Documentation & community resources

* [User Guide](https://actix.rs/docs/)
* [API Documentation (1.0)](https://docs.rs/actix-web/)
* [API Documentation (0.7)](https://docs.rs/actix-web/0.7.19/actix_web/)
* [Chat on gitter](https://gitter.im/actix/actix)
* Cargo package: [actix-web](https://crates.io/crates/actix-web)
* Minimum supported Rust version: 1.36 or later

## Example

```rust
use actix_web::{web, App, HttpServer, Responder};

fn index(info: web::Path<(u32, String)>) -> impl Responder {
    format!("Hello {}! id:{}", info.1, info.0)
}

fn main() -> std::io::Result<()> {
    HttpServer::new(
        || App::new().service(
              web::resource("/{id}/{name}/index.html").to(index)))
        .bind("127.0.0.1:8080")?
        .run()
}
```

### More examples

* [Basics](https://github.com/actix/examples/tree/master/basics/)
* [Stateful](https://github.com/actix/examples/tree/master/state/)
* [Multipart streams](https://github.com/actix/examples/tree/master/multipart/)
* [Simple websocket](https://github.com/actix/examples/tree/master/websocket/)
* [Tera](https://github.com/actix/examples/tree/master/template_tera/) /
  [Askama](https://github.com/actix/examples/tree/master/template_askama/) templates
* [Diesel integration](https://github.com/actix/examples/tree/master/diesel/)
* [r2d2](https://github.com/actix/examples/tree/master/r2d2/)
* [SSL / HTTP/2.0](https://github.com/actix/examples/tree/master/tls/)
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
