# Actix web [![Build Status](https://travis-ci.org/actix/actix-web.svg?branch=master)](https://travis-ci.org/actix/actix-web) [![Build status](https://ci.appveyor.com/api/projects/status/kkdb4yce7qhm5w85/branch/master?svg=true)](https://ci.appveyor.com/project/fafhrd91/actix-web-hdy9d/branch/master) [![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web) [![crates.io](http://meritbadge.herokuapp.com/actix-web)](https://crates.io/crates/actix-web)

Actix web is a small, fast, down-to-earth, open source rust web framework.

```rust,ignore
use actix_web::*;

fn index(req: HttpRequest) -> String {
    format!("Hello {}!", &req.match_info()["name"])
}

fn main() {
    HttpServer::new(
        || Application::new()
            .resource("/{name}", |r| r.f(index)))
        .bind("127.0.0.1:8080")?
        .start();
}
```

## Documentation

* [User Guide](http://actix.github.io/actix-web/guide/)
* [API Documentation (Development)](http://actix.github.io/actix-web/actix_web/)
* [API Documentation (Releases)](https://docs.rs/actix-web/)
* Cargo package: [actix-web](https://crates.io/crates/actix-web)
* Minimum supported Rust version: 1.20 or later

## Features

  * Supported *HTTP/1.x* and *HTTP/2.0* protocols
  * Streaming and pipelining
  * Keep-alive and slow requests handling
  * [WebSockets](https://actix.github.io/actix-web/actix_web/ws/index.html)
  * Transparent content compression/decompression (br, gzip, deflate)
  * Configurable request routing
  * Multipart streams
  * Middlewares ([Logger](https://actix.github.io/actix-web/guide/qs_10.html#logging), 
    [Session](https://actix.github.io/actix-web/guide/qs_10.html#user-sessions),
    [DefaultHeaders](https://actix.github.io/actix-web/guide/qs_10.html#default-headers))
  * Built on top of [Actix](https://github.com/actix/actix).

## Benchmarks

Some basic benchmarks could be found in this [respository](https://github.com/fafhrd91/benchmarks).

## Examples

* [Basic](https://github.com/actix/actix-web/tree/master/examples/basic.rs)
* [Stateful](https://github.com/actix/actix-web/tree/master/examples/state.rs)
* [Mulitpart streams](https://github.com/actix/actix-web/tree/master/examples/multipart/)
* [Simple websocket session](https://github.com/actix/actix-web/tree/master/examples/websocket.rs)
* [Tera templates](https://github.com/actix/actix-web/tree/master/examples/template_tera/)
* [Diesel integration](https://github.com/actix/actix-web/tree/master/examples/diesel/)
* [SSL / HTTP/2.0](https://github.com/actix/actix-web/tree/master/examples/tls/)
* [Tcp/Websocket chat](https://github.com/actix/actix-web/tree/master/examples/websocket-chat/)
* [SockJS Server](https://github.com/actix/actix-sockjs)

## License

This project is licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
   http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or
   http://opensource.org/licenses/MIT)

at your option.


[![Analytics](https://ga-beacon.appspot.com/UA-110322332-2/actix-web/readme)](https://github.com/igrigorik/ga-beacon)
