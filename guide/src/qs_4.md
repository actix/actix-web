# Handler

A request handler can by any object that implements
[`Handler` trait](../actix_web/dev/trait.Handler.html#implementors).
Request handling happen in two stages. First handler object get called. 
Handle can return any object that implements 
[*Responder trait*](../actix_web/trait.Responder.html#foreign-impls).
Then `respond_to()` get called on returned object. And finally
result of the `respond_to()` call get converted to `Reply` object.

By default actix provides `Responder` implementations for some standard types, 
like `&'static str`, `String`, etc.
For complete list of implementations check 
[Responder documentation](../actix_web/trait.Responder.html#foreign-impls).

Examples of valid handlers:

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

```rust,ignore
fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    ...
}
```

Some notes on shared application state and handler state. If you noticed
*Handler* trait is generic over *S*, which defines application state type. So
application state is accessible from handler with `HttpRequest::state()` method. 
But state is accessible as a read-only reference, if you need mutable access to state
you have to implement it yourself. On other hand handler can mutable access it's own state
as `handle` method takes mutable reference to *self*. Beware, actix creates multiple copies
of application state and handlers, unique for each thread, so if you run your
application in several threads actix will create same amount as number of threads 
of application state objects and handler objects.

Here is example of handler that stores number of processed requests:

```rust
# extern crate actix;
# extern crate actix_web;
use actix_web::*;
use actix_web::dev::Handler;

struct MyHandler(usize);

impl<S> Handler<S> for MyHandler {
    type Result = HttpResponse;

    /// Handle request
    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        self.0 += 1;
        httpcodes::HTTPOk.response()
    }
}
# fn main() {}
```

This handler will work, but `self.0` value will be different depends on number of threads and
number of requests processed per thread. Proper implementation would use `Arc` and `AtomicUsize`

```rust
# extern crate actix;
# extern crate actix_web;
use actix_web::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct MyHandler(Arc<AtomicUsize>);

impl<S> Handler<S> for MyHandler {
    type Result = HttpResponse;

    /// Handle request
    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        let num = self.0.load(Ordering::Relaxed) + 1;
        self.0.store(num, Ordering::Relaxed);
        httpcodes::HTTPOk.response()
    }
}

fn main() {
    let sys = actix::System::new("example");

    let inc = Arc::new(AtomicUsize::new(0));

    HttpServer::new(
        move || { 
            let cloned = inc.clone();
            Application::new()
            .resource("/", move |r| r.h(MyHandler(cloned)))
        })
        .bind("127.0.0.1:8088").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8088");
#    actix::Arbiter::system().send(actix::msgs::SystemExit(0));
    let _ = sys.run();
}
```

## Response with custom type

To return custom type directly from handler function type needs to implement `Responder` trait.
Let's create response for custom type that serializes to `application/json` response:

```rust
# extern crate actix;
# extern crate actix_web;
extern crate serde;
extern crate serde_json;
#[macro_use] extern crate serde_derive;
use actix_web::*;

#[derive(Serialize)]
struct MyObj {
    name: &'static str,
}

/// Responder
impl Responder for MyObj {
    type Item = HttpResponse;
    type Error = Error;

    fn respond_to(self, req: HttpRequest) -> Result<HttpResponse> {
        let body = serde_json::to_string(&self)?;

        // Create response and set content type
        Ok(HttpResponse::Ok()
            .content_type("application/json")
            .body(body)?)
    }
}

/// Because `MyObj` implements `Responder`, it is possible to return it directly
fn index(req: HttpRequest) -> MyObj {
    MyObj{name: "user"}
}

fn main() {
    let sys = actix::System::new("example");

    HttpServer::new(
        || Application::new()
            .resource("/", |r| r.method(Method::GET).f(index)))
        .bind("127.0.0.1:8088").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8088");
#    actix::Arbiter::system().send(actix::msgs::SystemExit(0));
    let _ = sys.run();
}
```

## Async handlers

There are two different types of async handlers. 

Response object could be generated asynchronously or more precisely, any type
that implements [*Responder*](../actix_web/trait.Responder.html) trait. In this case handle must
return `Future` object that resolves to *Responder* type, i.e:

```rust
# extern crate actix_web;
# extern crate futures;
# extern crate bytes;
# use actix_web::*;
# use bytes::Bytes;
# use futures::stream::once;
# use futures::future::{FutureResult, result};
fn index(req: HttpRequest) -> FutureResult<HttpResponse, Error> {

    result(HttpResponse::Ok()
           .content_type("text/html")
           .body(format!("Hello!"))
           .map_err(|e| e.into()))
}

fn index2(req: HttpRequest) -> FutureResult<&'static str, Error> {
    result(Ok("Welcome!"))
}

fn main() {
    Application::new()
        .resource("/async", |r| r.route().a(index))
        .resource("/", |r| r.route().a(index2))
        .finish();
}
```

Or response body can be generated asynchronously. In this case body
must implement stream trait `Stream<Item=Bytes, Error=Error>`, i.e:

```rust
# extern crate actix_web;
# extern crate futures;
# extern crate bytes;
# use actix_web::*;
# use bytes::Bytes;
# use futures::stream::once;
fn index(req: HttpRequest) -> HttpResponse {
    let body = once(Ok(Bytes::from_static(b"test")));

    HttpResponse::Ok()
       .content_type("application/json")
       .body(Body::Streaming(Box::new(body))).unwrap()
}

fn main() {
    Application::new()
        .resource("/async", |r| r.f(index))
        .finish();
}
```

Both methods could be combined. (i.e Async response with streaming body)
