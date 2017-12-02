# Handler

A request handler can by any object that implements
[`Handler` trait](../actix_web/struct.HttpResponse.html#implementations).

By default actix provdes several `Handler` implementations:

* Simple function that accepts `HttpRequest` and returns any object that 
  can be converted to `HttpResponse`
* Function that accepts `HttpRequest` and returns `Result<Reply, Into<Error>>` object.
* Function that accepts `HttpRequest` and return actor that has `HttpContext<A>`as a context. 

Actix provides response conversion into `HttpResponse` for some standard types, 
like `&'static str`, `String`, etc.
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

/// we have to convert Error into HttpResponse as well, but with 
/// specialization this could be handled genericly.
impl Into<HttpResponse> for MyObj {
    fn into(self) -> HttpResponse {
        let body = match serde_json::to_string(&self) {
            Err(err) => return Error::from(err).into(),
            Ok(body) => body,
        };

        // Create response and set content type
        HttpResponse::Ok()
            .content_type("application/json")
            .body(body).unwrap()
    }
}

fn main() {
    let sys = actix::System::new("example");

    HttpServer::new(
        Application::default("/")
            .resource("/", |r| r.handler(
                Method::GET, |req| {MyObj{name: "user".to_owned()}})))
        .serve::<_, ()>("127.0.0.1:8088").unwrap();

    println!("Started http server: 127.0.0.1:8088");
    actix::Arbiter::system().send(actix::msgs::SystemExit(0)); // <- remove this line, this code stops system during testing

    let _ = sys.run();
}
```

If `specialization` is enabled, conversion could be simplier:

```rust,ignore
impl Into<Result<HttpResponse>> for MyObj {
    fn into(self) -> Result<HttpResponse> {
        let body = serde_json::to_string(&self)?;

        Ok(HttpResponse::Ok()
            .content_type("application/json")
            .body(body)?)
    }
}
```

## Async handlers

There are two different types of async handlers. 

Response object could be generated asynchronously. In this case handle must
return `Future` object that resolves to `HttpResponse`, i.e:

```rust,ignore
fn index(req: HttpRequest) -> Box<Future<HttpResponse, Error>> {
   ...
}
```

This handler can be registered with `ApplicationBuilder::async()` and 
`Resource::async()` methods.

Or response body can be generated asynchronously. In this case body
must implement stream trait `Stream<Item=Bytes, Error=Error>`, i.e:


```rust,ignore
fn index(req: HttpRequest) -> HttpResponse {
    let body: Box<Stream<Item=Bytes, Error=Error>> = Box::new(SomeStream::new());

    HttpResponse::Ok().
       .content_type("application/json")
       .body(Body::Streaming(body)).unwrap()
}

fn main() {
    Application::default("/")
        .async("/async", index)
        .finish();
}
```

Both methods could be combined. (i.e Async response with streaming body)
