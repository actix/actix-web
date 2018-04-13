# Actix web [![Build Status](https://travis-ci.org/actix/actix-web.svg?branch=master)](https://travis-ci.org/actix/actix-web) [![Build status](https://ci.appveyor.com/api/projects/status/kkdb4yce7qhm5w85/branch/master?svg=true)](https://ci.appveyor.com/project/fafhrd91/actix-web-hdy9d/branch/master) [![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web) [![crates.io](https://meritbadge.herokuapp.com/actix-web)](https://crates.io/crates/actix-web) [![Join the chat at https://gitter.im/actix/actix](https://badges.gitter.im/actix/actix.svg)](https://gitter.im/actix/actix?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)

Actix web is a simple, pragmatic and extremely fast web framework for Rust.

* Supported *HTTP/1.x* and [*HTTP/2.0*](https://actix.rs/actix-web/guide/qs_13.html) protocols
* Streaming and pipelining
* Keep-alive and slow requests handling
* Client/server [WebSockets](https://actix.rs/actix-web/guide/qs_9.html) support
* Transparent content compression/decompression (br, gzip, deflate)
* Configurable [request routing](https://actix.rs/actix-web/guide/qs_5.html)
* Graceful server shutdown
* Multipart streams
* Static assets
* SSL support with OpenSSL or `native-tls`
* Middlewares ([Logger](https://actix.rs/actix-web/guide/qs_10.html#logging),
  [Session](https://actix.rs/actix-web/guide/qs_10.html#user-sessions),
  [Redis sessions](https://github.com/actix/actix-redis),
  [DefaultHeaders](https://actix.rs/actix-web/guide/qs_10.html#default-headers),
  [CORS](https://actix.rs/actix-web/actix_web/middleware/cors/index.html),
  [CSRF](https://actix.rs/actix-web/actix_web/middleware/csrf/index.html))
* Built on top of [Actix actor framework](https://github.com/actix/actix)

## Documentation & community resources

* [User Guide](https://actix.github.io/actix-web/guide/)
* [API Documentation (Development)](https://actix.github.io/actix-web/actix_web/)
* [API Documentation (Releases)](https://docs.rs/actix-web/)
* [Chat on gitter](https://gitter.im/actix/actix)
* Cargo package: [actix-web](https://crates.io/crates/actix-web)
* Minimum supported Rust version: 1.21 or later

## Example

```rust
extern crate actix_web;
use actix_web::{http, server, App, Path};

fn index(info: Path<(u32, String)>) -> String {
    format!("Hello {}! id:{}", info.0, info.1)
}

fn main() {
    server::new(
        || App::new()
            .route("/{id}/{name}/index.html", http::Method::GET, index))
        .bind("127.0.0.1:8080").unwrap()
        .run();
}
```

### More examples

* [Basics](https://github.com/actix/examples/tree/master/basics/)
* [Stateful](https://github.com/actix/examples/tree/master/state/)
* [Protobuf support](https://github.com/actix/examples/tree/master/protobuf/)
* [Multipart streams](https://github.com/actix/examples/tree/master/multipart/)
* [Simple websocket session](https://github.com/actix/examples/tree/master/websocket/)
* [Tera templates](https://github.com/actix/examples/tree/master/template_tera/)
* [Diesel integration](https://github.com/actix/examples/tree/master/diesel/)
* [SSL / HTTP/2.0](https://github.com/actix/examples/tree/master/tls/)
* [Tcp/Websocket chat](https://github.com/actix/examples/tree/master/websocket-chat/)
* [Json](https://github.com/actix/examples/tree/master/json/)

You may consider checking out
[this directory](https://github.com/actix/examples/tree/master/) for more examples.

## Benchmarks

* [TechEmpower Framework Benchmark](https://www.techempower.com/benchmarks/#section=data-r15&hw=ph&test=plaintext)

* Some basic benchmarks could be found in this [repository](https://github.com/fafhrd91/benchmarks).

## License

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))
* MIT license ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))

at your option.

## Code of Conduct

Contribution to the actix-web crate is organized under the terms of the
Contributor Covenant, the maintainer of actix-web, @fafhrd91, promises to
intervene to uphold that code of conduct.
