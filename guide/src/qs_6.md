# Application state

Application state is shared with all routes and resources within same application.
State could be accessed with `HttpRequest::state()` method as a read-only item
but interior mutability pattern with `RefCell` could be used to archive state mutability.
State could be accessed with `HttpContext::state()` in case of http actor. 
State also available to route matching predicates. State is not available
to application middlewares, middlewares receives `HttpRequest<()>` object.

Let's write simple application that uses shared state. We are going to store requests count
in the state: 
 
```rust
# extern crate actix;
# extern crate actix_web;
# 
use actix_web::*;
use std::cell::Cell;

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
    Application::with_state("/", AppState{counter: Cell::new(0)})
        .resource("/", |r| r.method(Method::GET).f(index))
        .finish();
}
```
