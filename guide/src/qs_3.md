# Overview

Actix web provides some primitives to build web servers and applications with Rust.
It provides routing, middlewares, pre-processing of requests, and post-processing of responses,
websocket protcol handling, multipart streams, etc.


## Application

All actix web server is built around `Application` instance.
It is used for registering handlers for routes and resources, middlewares.
Also it stores applicationspecific state that is shared accross all handlers 
within same application.

Application acts as namespace for all routes, i.e all routes for specific application
has same url path prefix:

```rust,ignore
   let app = Application::default("/prefix")
       .resource("/index.html", |r| r.handler(Method::GET, index)
       .finish()
```

In this example application with `/prefix` prefix and `index.html` resource
get created. This resource is available as on `/prefix/index.html` url.

### Application state

Application state is shared with all routes within same application.
State could be accessed with `HttpRequest::state()` method. It is read-only
but interior mutability pattern with `RefCell` could be used to archive state mutability.
State could be accessed with `HttpRequest::state()` method or 
`HttpContext::state()` in case of http actor.

Let's write simple application that uses shared state. We are going to store requests count
in the state: 
 
```rust
extern crate actix;
extern crate actix_web;

use std::cell::Cell;
use actix_web::*;

// This struct represents state
struct AppState {
    counter: Cell<usize>,
}

fn index(req: HttpRequest<AppState>) -> String {
    let count = req.state().counter.get() + 1; // <- get count
    req.state().counter.set(count);            // <- store new count in state

    format!("Request number: {}", count)      // <- response with count
}

fn main() {
    let sys = actix::System::new("example");

    HttpServer::new(
        Application::build("/", AppState{counter: Cell::new(0)})
            .resource("/", |r| r.handler(Method::GET, index)))
        .serve::<_, ()>("127.0.0.1:8088").unwrap();

    println!("Started http server: 127.0.0.1:8088");
    actix::Arbiter::system().send(actix::msgs::SystemExit(0)); // <- remove this line, this code stops system during testing

    let _ = sys.run();
}
```

## Handler

A request handler can have several different forms. 

* Simple function that accepts `HttpRequest` and returns `HttpResponse` or any 
   type that can be converted into `HttpResponse`. 
* Function that that accepts `HttpRequest` and returns `Stream<Item=Frame, Error=Error>`. 
* Http actor, i.e. actor that has `HttpContext<A>`as a context. 

Actix provides response conversion for some standard types, like `&'static str`, `String`, etc.
For complete list of implementations check 
[HttpResponse documentation](../actix_web/struct.HttpResponse.html#implementations).

Examples:

```rust,ignore
fn index(req: HttpRequest) -> &'static str {
    "Hello world!"
}
```

```rust,ignore
fn index(req: HttpRequest) -> String {
    "Hello world!".to_owned()
}
```

```rust,ignore
fn index(req: HttpRequest) -> Bytes {
    Bytes::from_static("Hello world!")
}
```
