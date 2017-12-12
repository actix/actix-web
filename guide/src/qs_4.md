# Handler

A request handler can by any object that implements
[`Handler` trait](../actix_web/dev/trait.Handler.html#implementors).
Request handling happen in two stages. First handler object get called. 
Handle can return any object that implements 
[`FromRequest` trait](../actix_web/trait.FromRequest.html#foreign-impls).
Then `from_request()` get called on returned object. And finally
result of the `from_request()` call get converted to `Reply` object.

By default actix provides several `FromRequest` implementations for some standard types, 
like `&'static str`, `String`, etc.
For complete list of implementations check 
[FromRequest documentation](../actix_web/trait.FromRequest.html#foreign-impls).

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

## Response with custom type

To return custom type directly from handler function `FromResponse` trait should be 
implemented for this type. Let's create response for custom type that 
serializes to `application/json` response:

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

/// we have to convert Error into HttpResponse as well
impl FromRequest for MyObj {
    type Item = HttpResponse;
    type Error = Error;

    fn from_request(self, req: HttpRequest) -> Result<HttpResponse> {
        let body = serde_json::to_string(&self)?;

        // Create response and set content type
        Ok(HttpResponse::Ok()
            .content_type("application/json")
            .body(body)?)
    }
}

fn index(req: HttpRequest) -> MyObj {
    MyObj{name: "user"}
}

fn main() {
    let sys = actix::System::new("example");

    HttpServer::new(
        || Application::new()
            .resource("/", |r| r.method(Method::GET).f(index)))
        .serve::<_, ()>("127.0.0.1:8088").unwrap();

    println!("Started http server: 127.0.0.1:8088");
#    actix::Arbiter::system().send(actix::msgs::SystemExit(0));
    let _ = sys.run();
}
```

## Async handlers

There are two different types of async handlers. 

Response object could be generated asynchronously. In this case handle must
return `Future` object that resolves to `HttpResponse`, i.e:

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

fn main() {
    Application::new()
        .resource("/async", |r| r.route().a(index))
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
