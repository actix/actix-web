# Actix web [![Build Status](https://travis-ci.org/actix/actix-web.svg?branch=master)](https://travis-ci.org/actix/actix-web) [![Build status](https://ci.appveyor.com/api/projects/status/kkdb4yce7qhm5w85/branch/master?svg=true)](https://ci.appveyor.com/project/fafhrd91/actix-web-hdy9d/branch/master) [![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web) [![crates.io](http://meritbadge.herokuapp.com/actix-web)](https://crates.io/crates/actix-web)

Actix web is a small, fast, down-to-earth, open source rust web framework.

```rust,ignore
extern crate actix_web;
use actix_web::*;

fn index(req: HttpRequest) -> String {
    format!("Hello {}!", &req.match_info()["name"])
}

fn main() {
    HttpServer::new(
        Application::default("/")
            .resource("/{name}", |r| r.method(Method::GET).f(index)))
        .serve::<_, ()>("127.0.0.1:8080");
}
```

## Documentation

* [User Guide](http://actix.github.io/actix-web/guide/)
* [API Documentation (Development)](http://actix.github.io/actix-web/actix_web/)
* [API Documentation (Releases)](https://docs.rs/actix-web/)
* Cargo package: [actix-web](https://crates.io/crates/actix-web)
* Minimum supported Rust version: 1.20 or later

## Features

  * Supported HTTP/1 and HTTP/2 protocols
  * Streaming and pipelining
  * Keep-alive and slow requests handling
  * [WebSockets](https://actix.github.io/actix-web/actix_web/ws/index.html)
  * Transparent content compression/decompression (br, gzip, deflate)
  * Configurable request routing
  * Multipart streams
  * Middlewares (Logger, Session included)
  * Built on top of [Actix](https://github.com/actix/actix).

## HTTP/2

Actix web automatically upgrades connection to `http/2` if possible.

### Negotiation

`HTTP/2` protocol over tls without prior knowlage requires
[tls alpn](https://tools.ietf.org/html/rfc7301). At the moment only
`rust-openssl` supports alpn.

```toml
[dependencies]
actix-web = { git = "https://github.com/actix/actix-web", features=["alpn"] }
```

Upgrade to `http/2` schema described in
[rfc section 3.2](https://http2.github.io/http2-spec/#rfc.section.3.2) is not supported.
Starting `http/2` with prior knowledge is supported for both clear text connection
and tls connection. [rfc section 3.4](https://http2.github.io/http2-spec/#rfc.section.3.4)

[tls example](https://github.com/actix/actix-web/tree/master/examples/tls)

## Examples

* [Basic](https://github.com/actix/actix-web/tree/master/examples/basic.rs)
* [Stateful](https://github.com/actix/actix-web/tree/master/examples/state.rs)
* [Mulitpart streams](https://github.com/actix/actix-web/tree/master/examples/multipart)
* [Simple websocket session](https://github.com/actix/actix-web/tree/master/examples/websocket.rs)
* [Tcp/Websocket chat](https://github.com/actix/actix-web/tree/master/examples/websocket-chat)
* [SockJS Server](https://github.com/actix/actix-sockjs)

## License

Actix web is licensed under the [Apache-2.0 license](http://opensource.org/licenses/APACHE-2.0).
