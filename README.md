# Actix web [![Build Status](https://travis-ci.org/actix/actix-web.svg?branch=master)](https://travis-ci.org/actix/actix-web) [![Build Status](https://ci.appveyor.com/api/projects/status/github/fafhrd91/actix-web-hdy9d?branch=master&svg=true)](https://ci.appveyor.com/project/fafhrd91/actix-web-hdy9d) [![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web)

Web framework for [Actix](https://github.com/actix/actix).

* [API Documentation](http://actix.github.io/actix-web/actix_web/)
* Cargo package: [actix-http](https://crates.io/crates/actix-web)
* Minimum supported Rust version: 1.20 or later

---

Actix web is licensed under the [Apache-2.0 license](http://opensource.org/licenses/APACHE-2.0).

## Features

  * HTTP 1.1 and 1.0 support
  * Streaming and pipelining support
  * Keep-alive and slow requests support
  * [WebSockets support](https://actix.github.io/actix-web/actix_web/ws/index.html)
  * Configurable request routing
  * Multipart streams
  * Middlewares

## Usage

To use `actix-web`, add this to your `Cargo.toml`:

```toml
[dependencies]
actix-web = { git = "https://github.com/actix/actix-web.git" }
```

## Example

* [Mulitpart support](https://github.com/actix/actix-web/tree/master/examples/multipart)
* [Simple websocket example](https://github.com/actix/actix-web/tree/master/examples/websocket.rs)
* [Tcp/Websocket chat](https://github.com/actix/actix-web/tree/master/examples/websocket-chat)


```rust
extern crate actix;
extern crate actix_web;
extern crate futures;

use actix::*;
use actix_web::*;

fn main() {
    let system = System::new("test");

    // start http server
    HttpServer::new(
        // create application
        Application::default("/")
            .resource("/", |r|
                r.handler(Method::GET, |req, payload, state| {
                    httpcodes::HTTPOk
                })
             )
             .finish())
        .serve::<_, ()>("127.0.0.1:8080").unwrap();

    // stop system
    Arbiter::handle().spawn_fn(|| {
        Arbiter::system().send(msgs::SystemExit(0));
        futures::future::ok(())
    });

    system.run();
}
```
