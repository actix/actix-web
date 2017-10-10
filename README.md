# Actix http [![Build Status](https://travis-ci.org/fafhrd91/actix-http.svg?branch=master)](https://travis-ci.org/fafhrd91/actix-http)

Actix http is a server http framework for Actix framework.

* [API Documentation](http://fafhrd91.github.io/actix-http/actix_http/)
* Cargo package: [actix-http](https://crates.io/crates/actix-http)
* Minimum supported Rust version: 1.20 or later

---

Actix http is licensed under the [Apache-2.0 license](http://opensource.org/licenses/APACHE-2.0).

## Features

  * HTTP 1.1 and 1.0 support
  * Streaming and pipelining support
  * [WebSockets support](https://fafhrd91.github.io/actix-http/actix_http/ws/index.html)
  * [Configurable request routing](https://fafhrd91.github.io/actix-http/actix_http/struct.RoutingMap.html)

## Usage

To use `actix-http`, add this to your `Cargo.toml`:

```toml
[dependencies]
actix-http = { git = "https://github.com/fafhrd91/actix-http.git" }
```

## Example

```rust
extern crate actix;
extern crate actix_http;
extern crate futures;
use std::net;
use std::str::FromStr;

use actix::prelude::*;
use actix_http::*;

// Route
struct MyRoute;

impl Actor for MyRoute {
    type Context = HttpContext<Self>;
}

impl Route for MyRoute {
    type State = ();

    fn request(req: HttpRequest, payload: Payload, ctx: &mut HttpContext<Self>) -> Reply<Self>
    {
        Reply::reply(req, httpcodes::HTTPOk)
    }
}

fn main() {
    let system = System::new("test");

    // create routing map with `MyRoute` route
    let mut routes = RoutingMap::default();
    routes
      .add_resource("/")
        .post::<MyRoute>();

    // start http server
    let http = HttpServer::new(routes);
    http.serve::<()>(
        &net::SocketAddr::from_str("127.0.0.1:8880").unwrap()).unwrap();

    // stop system
    Arbiter::handle().spawn_fn(|| {
        Arbiter::system().send(msgs::SystemExit(0));
        futures::future::ok(())
    });

    system.run();
    println!("Done");
}
```
