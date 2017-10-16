# Actix web [![Build Status](https://travis-ci.org/fafhrd91/actix-web.svg?branch=master)](https://travis-ci.org/fafhrd91/actix-web) [![codecov](https://codecov.io/gh/fafhrd91/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/fafhrd91/actix-web)

Web framework for Actix.

* [API Documentation](http://fafhrd91.github.io/actix-web/actix_web/)
* Cargo package: [actix-http](https://crates.io/crates/actix-web)
* Minimum supported Rust version: 1.20 or later

---

Actix web is licensed under the [Apache-2.0 license](http://opensource.org/licenses/APACHE-2.0).

## Features

  * HTTP 1.1 and 1.0 support
  * Streaming and pipelining support
  * Keep-alive and slow requests support
  * [WebSockets support](https://fafhrd91.github.io/actix-web/actix_web/ws/index.html)
  * [Configurable request routing](https://fafhrd91.github.io/actix-web/actix_web/struct.RoutingMap.html)

## Usage

To use `actix-web`, add this to your `Cargo.toml`:

```toml
[dependencies]
actix-web = { git = "https://github.com/fafhrd91/actix-web.git" }
```

## Example

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
        // create routing map
        RoutingMap::default()
            // handler for "GET /"
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
