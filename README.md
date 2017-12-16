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

  * Supported *HTTP/1.x* and *HTTP/2.0* protocols
  * Streaming and pipelining
  * Keep-alive and slow requests handling
  * [WebSockets](https://actix.github.io/actix-web/actix_web/ws/index.html)
  * Transparent content compression/decompression (br, gzip, deflate)
  * Configurable request routing
  * Multipart streams
  * Middlewares (Logger, Session, DefaultHeaders)
  * Built on top of [Actix](https://github.com/actix/actix).

## Benchmarks

This is totally unscientific and probably pretty useless. In real world business
logic would dominate on performance side. But in any case. i took several web frameworks
for rust and used theirs *hello world* example. All projects are compiled with
`--release` parameter. I didnt test single thread performance for iron and rocket.
As a testing tool i used `wrk` and following commands

`wrk -t20 -c100 -d10s http://127.0.0.1:8080/`

`wrk -t20 -c100 -d10s http://127.0.0.1:8080/ -s ./pipeline.lua --latency -- / 128`

I ran all tests on localhost on MacBook Pro late 2017. It has 4 cpu and 8 logical cpus.
Each result is best of five runs. All measurements are req/sec.

Name | 1 thread | 1 pipeline | 3 thread | 3 pipeline | 8 thread | 8 pipeline
---- | -------- | ---------- | -------- | ---------- | -------- | ----------
Actix | 91.200 | 912.000 | 122.100 | 2.083.000 | 107.400 | 2.650.000
Gotham | 61.000 | 178.000 |   |   |   |
Iron |   |   |   |   | 94.500 | 78.000
Rocket |   |   |   |   | 95.500 | failed
Shio | 71.800 | 317.800 |   |   |   |   |
tokio-minihttp | 106.900 | 1.047.000 |   |   |   |

Some notes on results. Iron and Rocket got tested with 8 threads,
which showed best results. Gothan and tokio-minihttp seem does not support
multithreading, or at least i couldn't figured out. I manually enabled pipelining
for *Shio* and Gotham*. While shio seems support multithreading, but it showed
absolutly same results for any how number of threads (maybe macos problem?)
Rocket completely failed in pipelined tests.

## Examples

* [Basic](https://github.com/actix/actix-web/tree/master/examples/basic.rs)
* [Stateful](https://github.com/actix/actix-web/tree/master/examples/state.rs)
* [Mulitpart streams](https://github.com/actix/actix-web/tree/master/examples/multipart)
* [Simple websocket session](https://github.com/actix/actix-web/tree/master/examples/websocket.rs)
* [Tcp/Websocket chat](https://github.com/actix/actix-web/tree/master/examples/websocket-chat)
* [SockJS Server](https://github.com/actix/actix-sockjs)

## License

Actix web is licensed under the [Apache-2.0 license](http://opensource.org/licenses/APACHE-2.0).
