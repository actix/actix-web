# Application state

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
    Application::build("/", AppState{counter: Cell::new(0)})
        .resource("/", |r| r.handler(Method::GET, index))
        .finish();
}
```
