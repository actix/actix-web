# Application

Actix web provides various primitives to build web servers and applications with Rust.
It provides routing, middlewares, pre-processing of requests, post-processing of responses,
websocket protocol handling, multipart streams, etc.

All actix web servers are built around the `App` instance.
It is used for registering routes for resources and middlewares.
It also stores application state shared across all handlers within same application.

Applications act as a namespace for all routes, i.e all routes for a specific application
have the same url path prefix. The application prefix always contains a leading "/" slash.
If a supplied prefix does not contain leading slash, it is automatically inserted.
The prefix should consist of value path segments.

> For an application with prefix `/app`,
> any request with the paths `/app`, `/app/`, or `/app/test` would match;
> however, the path `/application` would not match.

```rust,ignore
# extern crate actix_web;
# extern crate tokio_core;
# use actix_web::{*, http::Method};
# fn index(req: HttpRequest) -> &'static str {
#    "Hello world!"
# }
# fn main() {
   let app = App::new()
       .prefix("/app")
       .resource("/index.html", |r| r.method(Method::GET).f(index))
       .finish()
# }
```

In this example, an application with the `/app` prefix and a `index.html` resource
are created. This resource is available through the `/app/index.html` url.

> For more information, check the
> [URL Dispatch](./qs_5.html#using-a-application-prefix-to-compose-applications) section.

Multiple applications can be served with one server:

```rust
# extern crate actix_web;
# extern crate tokio_core;
# use tokio_core::net::TcpStream;
# use std::net::SocketAddr;
use actix_web::{server, App, HttpResponse};

fn main() {
    server::new(|| vec![
        App::new()
            .prefix("/app1")
            .resource("/", |r| r.f(|r| HttpResponse::Ok())),
        App::new()
            .prefix("/app2")
            .resource("/", |r| r.f(|r| HttpResponse::Ok())),
        App::new()
            .resource("/", |r| r.f(|r| HttpResponse::Ok())),
    ]);
}
```

All `/app1` requests route to the first application, `/app2` to the second, and all other to the third.
**Applications get matched based on registration order**. If an application with a more generic
prefix is registered before a less generic one, it would effectively block the less generic
application matching. For example, if an `App` with the prefix `"/"` was registered
as the first application, it would match all incoming requests.

## State

Application state is shared with all routes and resources within the same application.
When using an http actor,state can be accessed with the `HttpRequest::state()` as read-only,
but interior mutability with `RefCell` can be used to achieve state mutability.
State is also available for route matching predicates and middlewares.

Let's write a simple application that uses shared state. We are going to store request count
in the state:

```rust
# extern crate actix;
# extern crate actix_web;
#
use std::cell::Cell;
use actix_web::{App, HttpRequest, http};

// This struct represents state
struct AppState {
    counter: Cell<usize>,
}

fn index(req: HttpRequest<AppState>) -> String {
    let count = req.state().counter.get() + 1; // <- get count
    req.state().counter.set(count);            // <- store new count in state

    format!("Request number: {}", count)       // <- response with count
}

fn main() {
    App::with_state(AppState{counter: Cell::new(0)})
        .resource("/", |r| r.method(http::Method::GET).f(index))
        .finish();
}
```

> **Note**: http server accepts an application factory rather than an application
> instance. Http server constructs an application instance for each thread, thus application state
> must be constructed multiple times. If you want to share state between different threads, a
> shared object should be used, e.g. `Arc`. Application state does not need to be `Send` and `Sync`,
> but the application factory must be `Send` + `Sync`.
