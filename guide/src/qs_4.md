# Handler

A request handler can by any object that implements
[`Handler` trait](../actix_web/struct.HttpResponse.html#implementations).

By default actix provdes several `Handler` implementations:

* Simple function that accepts `HttpRequest` and returns any object that 
  implements `FromRequest` trait
* Function that accepts `HttpRequest` and returns `Result<Reply, Into<Error>>` object.
* Function that accepts `HttpRequest` and return actor that has `HttpContext<A>`as a context. 

Actix provides response `FromRequest` implementation for some standard types, 
like `&'static str`, `String`, etc.
For complete list of implementations check 
[FromRequest documentation](../actix_web/trait.FromRequest.html#foreign-impls).

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

```rust,ignore
fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    ...
}
```

## Custom conversion

Let's create response for custom type that serializes to `application/json` response:

```rust
extern crate actix;
extern crate actix_web;
extern crate serde;
extern crate serde_json;
#[macro_use] extern crate serde_derive;
use actix_web::*;

#[derive(Serialize)]
struct MyObj {
    name: String,
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

fn main() {
    let sys = actix::System::new("example");

    HttpServer::new(
        Application::default("/")
            .resource("/", |r| r.method(
                Method::GET).f(|req| {MyObj{name: "user".to_owned()}})))
        .serve::<_, ()>("127.0.0.1:8088").unwrap();

    println!("Started http server: 127.0.0.1:8088");
    actix::Arbiter::system().send(actix::msgs::SystemExit(0)); // <- remove this line, this code stops system during testing

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
    Application::default("/")
        .route("/async", |r| r.a(index))
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
    Application::default("/")
        .route("/async", |r| r.f(index))
        .finish();
}
```

Both methods could be combined. (i.e Async response with streaming body)
