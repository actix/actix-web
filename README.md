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
        .serve("127.0.0.1:8080");
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

This is totally unscientific and probably pretty useless. In real world, business
logic would dominate on performance side. I took several web frameworks
for rust and used *hello world* examples for tests. All projects are compiled with
`--release` parameter. I didnt test single thread performance for *iron* and *rocket*.
As a testing tool i used `wrk` and following commands

`wrk -t20 -c100 -d10s http://127.0.0.1:8080/`

`wrk -t20 -c100 -d10s http://127.0.0.1:8080/ -s ./pipeline.lua --latency -- / 128`

I ran all tests on my MacBook Pro with 2.9Gh i7 with 4 physical cpus and 8 logical cpus.
Each result is best of five runs. All measurements are *req/sec*.

Name | 1 thread | 1 pipeline | 3 thread | 3 pipeline | 8 thread | 8 pipeline
---- | -------- | ---------- | -------- | ---------- | -------- | ----------
Actix | 91.200 | 950.000 | 122.100 | 2.083.000 | 107.400 | 2.730.000
Gotham | 61.000 | 178.000 |   |   |   |
Iron |   |   |   |   | 94.500 | 78.000
Rocket |   |   |   |   | 95.500 | failed
Shio | 71.800 | 317.800 |   |   |   |   |
tokio-minihttp | 106.900 | 1.047.000 |   |   |   |

I got best performance for sync frameworks with 8 threads, other number of 
threads always gave me worse performance. *Iron* could handle piplined 
requests with lower performace. Interestingly, *Rocket* completely failed in pipelined test.
*Gothan* seems does not support multithreading, or at least i couldn't figured out. 
I manually enabled pipelining for *Shio* and *Gotham*. While *shio* seems support 
multithreading, but it result absolutly same results for any how number of threads
(maybe macos problem?).

## Examples

* [Basic](https://github.com/actix/actix-web/tree/master/examples/basic.rs)
* [Stateful](https://github.com/actix/actix-web/tree/master/examples/state.rs)
* [Mulitpart streams](https://github.com/actix/actix-web/tree/master/examples/multipart)
* [Simple websocket session](https://github.com/actix/actix-web/tree/master/examples/websocket.rs)
* [Tcp/Websocket chat](https://github.com/actix/actix-web/tree/master/examples/websocket-chat)
* [SockJS Server](https://github.com/actix/actix-sockjs)

## License

Actix web is licensed under the [Apache-2.0 license](http://opensource.org/licenses/APACHE-2.0).
