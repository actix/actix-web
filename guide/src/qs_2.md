# Getting Started

Let’s create and run our first actix web application. We’ll create a new Cargo project
that depends on actix web and then run the application.

In previous section we already installed required rust version. Now let's create new cargo projects.

## Hello, world! 

Let’s write our first actix web application! Start by creating a new binary-based
Cargo project and changing into the new directory:

```bash
cargo new hello-world --bin
cd hello-world
```

Now, add actix and actix web as dependencies of your project by ensuring your Cargo.toml 
contains the following:

```toml
[dependencies]
actix = "0.4"
actix-web = "0.3"
```

In order to implement a web server, first we need to create a request handler.

A request handler is a function that accepts a `HttpRequest` instance as its only parameter 
and returns a type that can be converted into `HttpResponse`:

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
   Application::new()
       .resource("/", |r| r.f(index));
# }
```

After that, application instance can be used with `HttpServer` to listen for incoming
connections. Server accepts function that should return `HttpHandler` instance:

```rust,ignore
   HttpServer::new(
       || Application::new()
           .resource("/", |r| r.f(index)))
       .bind("127.0.0.1:8088")?
       .run();
```

That's it. Now, compile and run the program with cargo run. 
Head over to ``http://localhost:8088/`` to see the results.

Here is full source of main.rs file:

```rust
# use std::thread;
# extern crate actix_web;
use actix_web::*;

fn index(req: HttpRequest) -> &'static str {
    "Hello world!"
}

fn main() {
# let child = thread::spawn(|| {
    HttpServer::new(
        || Application::new()
            .resource("/", |r| r.f(index)))
        .bind("127.0.0.1:8088").expect("Can not bind to 127.0.0.1:8088")
        .run();
  });
# child.join().expect("failed to join server thread");
}
```

Note on `actix` crate. Actix web framework is built on top of actix actor library. 
`actix::System` initializes actor system, `HttpServer` is an actor and must run within
properly configured actix system. For more information please check
[actix documentation](https://actix.github.io/actix/actix/)
