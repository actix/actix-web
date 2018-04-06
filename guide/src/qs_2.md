# Getting Started

Let’s write our first actix web application!

## Hello, world!

Start by creating a new binary-based Cargo project and changing into the new directory:

```bash
cargo new hello-world --bin
cd hello-world
```

Now, add actix and actix web as dependencies of your project by ensuring your Cargo.toml
contains the following:

```toml
[dependencies]
actix = "0.5"
actix-web = "0.4"
```

In order to implement a web server, we first need to create a request handler.

A request handler is a function that accepts an `HttpRequest` instance as its only parameter
and returns a type that can be converted into `HttpResponse`:

Filename: src/main.rs
```rust
# extern crate actix_web;
# use actix_web::*;
  fn index(req: HttpRequest) -> &'static str {
      "Hello world!"
  }
# fn main() {}
```

Next, create an `Application` instance and register the
request handler with the application's `resource` on a particular *HTTP method* and *path*::

```rust
# extern crate actix_web;
# use actix_web::*;
# fn index(req: HttpRequest) -> &'static str {
#    "Hello world!"
# }
# fn main() {
   App::new()
       .resource("/", |r| r.f(index));
# }
```

After that, the application instance can be used with `HttpServer` to listen for incoming
connections. The server accepts a function that should return an `HttpHandler` instance:

```rust,ignore
   HttpServer::new(
       || App::new()
           .resource("/", |r| r.f(index)))
       .bind("127.0.0.1:8088")?
       .run();
```

That's it! Now, compile and run the program with `cargo run`.
Head over to ``http://localhost:8088/`` to see the results.

The full source of src/main.rs is listed below:

```rust
# use std::thread;
extern crate actix_web;
use actix_web::{server, App, HttpRequest, HttpResponse};

fn index(req: HttpRequest) -> &'static str {
    "Hello world!"
}

fn main() {
#  // In the doctest suite we can't run blocking code - deliberately leak a thread
#  // If copying this example in show-all mode, make sure you skip the thread spawn
#  // call.
#  thread::spawn(|| {
    server::HttpServer::new(
        || App::new()
            .resource("/", |r| r.f(index)))
        .bind("127.0.0.1:8088").expect("Can not bind to 127.0.0.1:8088")
        .run();
#  });
}
```

> **Note**: actix web is built upon [actix](https://github.com/actix/actix),
> an [actor model](https://en.wikipedia.org/wiki/Actor_model) framework in Rust.

`actix::System` initializes actor system, `HttpServer` is an actor and must run within a
properly configured actix system.

> For more information, check out the [actix documentation](https://actix.github.io/actix/actix/)
> and [actix guide](https://actix.github.io/actix/guide/).
