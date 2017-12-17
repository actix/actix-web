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
actix = "0.3"
actix-web = { git = "https://github.com/actix/actix-web" }
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
   let app = Application::new()
       .resource("/", |r| r.f(index))
       .finish();
# }
```

After that, application instance can be used with `HttpServer` to listen for incoming
connections. Server accepts function that should return `HttpHandler` instance:

```rust,ignore
   HttpServer::new(|| app).serve::<_, ()>("127.0.0.1:8088");
```

That's it. Now, compile and run the program with cargo run. 
Head over to ``http://localhost:8088/`` to see the results.

Here is full source of main.rs file:

```rust
extern crate actix;
extern crate actix_web;
use actix_web::*;

fn index(req: HttpRequest) -> &'static str {
    "Hello world!"
}

fn main() {
    let sys = actix::System::new("example");

    HttpServer::new(
        || Application::new()
            .resource("/", |r| r.f(index)))
        .bind("127.0.0.1:8088").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8088");
#     actix::Arbiter::system().send(actix::msgs::SystemExit(0));
    let _ = sys.run();
}
```

Note on `actix` crate. Actix web framework is built on top of actix actor library. 
`actix::System` initializes actor system, `HttpServer` is an actor and must run within
proper configured actix system. For more information please check
[actix documentation](https://actix.github.io/actix/actix/)
