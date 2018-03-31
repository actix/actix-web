# Handler

A request handler can be any object that implements
[*Handler trait*](../actix_web/dev/trait.Handler.html).
Request handling happens in two stages. First the handler object is called.
Handler can return any object that implements
[*Responder trait*](../actix_web/trait.Responder.html#foreign-impls).
Then `respond_to()` is called on the returned object. And finally
result of the `respond_to()` call is converted to a `Reply` object.

By default actix provides `Responder` implementations for some standard types,
like `&'static str`, `String`, etc.
For a complete list of implementations, check
[*Responder documentation*](../actix_web/trait.Responder.html#foreign-impls).

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
application state is accessible from handler with the `HttpRequest::state()` method.
But state is accessible as a read-only reference - if you need mutable access to state
you have to implement it yourself. On other hand, handler can mutably access its own state
as the `handle` method takes a mutable reference to *self*. Beware, actix creates multiple copies
of application state and handlers, unique for each thread, so if you run your
application in several threads, actix will create the same amount as number of threads
of application state objects and handler objects.

Here is an example of a handler that stores the number of processed requests:

```rust
# extern crate actix_web;
use actix_web::{App, HttpRequest, HttpResponse, dev::Handler};

struct MyHandler(usize);

impl<S> Handler<S> for MyHandler {
    type Result = HttpResponse;

    /// Handle request
    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        self.0 += 1;
        HttpResponse::Ok().into()
    }
}
# fn main() {}
```

This handler will work, but `self.0` will be different depending on the number of threads and
number of requests processed per thread. A proper implementation would use `Arc` and `AtomicUsize`

```rust
# extern crate actix;
# extern crate actix_web;
use actix_web::{server, App, HttpRequest, HttpResponse, dev::Handler};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct MyHandler(Arc<AtomicUsize>);

impl<S> Handler<S> for MyHandler {
    type Result = HttpResponse;

    /// Handle request
    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        self.0.fetch_add(1, Ordering::Relaxed);
        HttpResponse::Ok().into()
    }
}

fn main() {
    let sys = actix::System::new("example");

    let inc = Arc::new(AtomicUsize::new(0));

    server::new(
        move || {
            let cloned = inc.clone();
            App::new()
                .resource("/", move |r| r.h(MyHandler(cloned)))
        })
        .bind("127.0.0.1:8088").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8088");
#    actix::Arbiter::system().do_send(actix::msgs::SystemExit(0));
    let _ = sys.run();
}
```

Be careful with synchronization primitives like *Mutex* or *RwLock*. Actix web framework
handles requests asynchronously; by blocking thread execution all concurrent
request handling processes would block. If you need to share or update some state
from multiple threads consider using the [actix](https://actix.github.io/actix/actix/) actor system.

## Response with custom type

To return a custom type directly from a handler function, the type needs to implement  the `Responder` trait.
Let's create a response for a custom type that serializes to an `application/json` response:

```rust
# extern crate actix;
# extern crate actix_web;
extern crate serde;
extern crate serde_json;
#[macro_use] extern crate serde_derive;
use actix_web::{App, HttpServer, HttpRequest, HttpResponse, Error, Responder, http};

#[derive(Serialize)]
struct MyObj {
    name: &'static str,
}

/// Responder
impl Responder for MyObj {
    type Item = HttpResponse;
    type Error = Error;

    fn respond_to(self, req: HttpRequest) -> Result<HttpResponse, Error> {
        let body = serde_json::to_string(&self)?;

        // Create response and set content type
        Ok(HttpResponse::Ok()
            .content_type("application/json")
            .body(body))
    }
}

/// Because `MyObj` implements `Responder`, it is possible to return it directly
fn index(req: HttpRequest) -> MyObj {
    MyObj{name: "user"}
}

fn main() {
    let sys = actix::System::new("example");

    HttpServer::new(
        || App::new()
            .resource("/", |r| r.method(http::Method::GET).f(index)))
        .bind("127.0.0.1:8088").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8088");
#    actix::Arbiter::system().do_send(actix::msgs::SystemExit(0));
    let _ = sys.run();
}
```

## Async handlers

There are two different types of async handlers.

Response objects can be generated asynchronously or more precisely, any type
that implements the [*Responder*](../actix_web/trait.Responder.html) trait. In this case the handler must return a `Future` object that resolves to the *Responder* type, i.e:

```rust
# extern crate actix_web;
# extern crate futures;
# extern crate bytes;
# use actix_web::*;
# use bytes::Bytes;
# use futures::stream::once;
# use futures::future::{Future, result};
fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {

    result(Ok(HttpResponse::Ok()
              .content_type("text/html")
              .body(format!("Hello!"))))
           .responder()
}

fn index2(req: HttpRequest) -> Box<Future<Item=&'static str, Error=Error>> {
    result(Ok("Welcome!"))
        .responder()
}

fn main() {
    App::new()
        .resource("/async", |r| r.route().a(index))
        .resource("/", |r| r.route().a(index2))
        .finish();
}
```

Or the response body can be generated asynchronously. In this case body
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
       .body(Body::Streaming(Box::new(body)))
}

fn main() {
    App::new()
        .resource("/async", |r| r.f(index))
        .finish();
}
```

Both methods can be combined. (i.e Async response with streaming body)

It is possible to return a `Result` where the `Result::Item` type can be `Future`.
In this example the `index` handler can return an error immediately or return a
future that resolves to a `HttpResponse`.

```rust
# extern crate actix_web;
# extern crate futures;
# extern crate bytes;
# use actix_web::*;
# use bytes::Bytes;
# use futures::stream::once;
# use futures::future::{Future, result};
fn index(req: HttpRequest) -> Result<Box<Future<Item=HttpResponse, Error=Error>>, Error> {
    if is_error() {
       Err(error::ErrorBadRequest("bad request"))
    } else {
       Ok(Box::new(
           result(Ok(HttpResponse::Ok()
                  .content_type("text/html")
                  .body(format!("Hello!"))))))
    }
}
#
# fn is_error() -> bool { true }
# fn main() {
#     App::new()
#        .resource("/async", |r| r.route().f(index))
#        .finish();
# }
```

## Different return types (Either)

Sometimes you need to return different types of responses. For example
you can do error check and return error and return async response otherwise.
Or any result that requires two different types.
For this case the [*Either*](../actix_web/enum.Either.html) type can be used.
*Either* allows combining two different responder types into a single type.

```rust
# extern crate actix_web;
# extern crate futures;
# use actix_web::*;
# use futures::future::Future;
use futures::future::result;
use actix_web::{Either, Error, HttpResponse};

type RegisterResult = Either<HttpResponse, Box<Future<Item=HttpResponse, Error=Error>>>;

fn index(req: HttpRequest) -> RegisterResult {
    if is_a_variant() { // <- choose variant A
        Either::A(
            HttpResponse::BadRequest().body("Bad data"))
    } else {
        Either::B(      // <- variant B
            result(Ok(HttpResponse::Ok()
                   .content_type("text/html")
                   .body(format!("Hello!")))).responder())
    }
}
# fn is_a_variant() -> bool { true }
# fn main() {
#    App::new()
#        .resource("/register", |r| r.f(index))
#        .finish();
# }
```

## Tokio core handle

Any actix web handler runs within a properly configured
[actix system](https://actix.github.io/actix/actix/struct.System.html)
and [arbiter](https://actix.github.io/actix/actix/struct.Arbiter.html).
You can always get access to the tokio handle via the
[Arbiter::handle()](https://actix.github.io/actix/actix/struct.Arbiter.html#method.handle)
method.
