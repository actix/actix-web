# Testing

Every application should be well tested and. Actix provides the tools to perform unit and
integration tests.

## Unit tests

For unit testing actix provides request builder type and simple handler runner.
[*TestRequest*](../actix_web/test/struct.TestRequest.html) implements builder-like pattern.
You can generate `HttpRequest` instance with `finish()` method or you can
run your handler with `run()` or `run_async()` methods.

```rust
# extern crate http;
# extern crate actix_web;
use http::{header, StatusCode};
use actix_web::*;
use actix_web::test::TestRequest;

fn index(req: HttpRequest) -> HttpResponse {
     if let Some(hdr) = req.headers().get(header::CONTENT_TYPE) {
        if let Ok(s) = hdr.to_str() {
            return httpcodes::HTTPOk.into()
        }
     }
     httpcodes::HTTPBadRequest.into()
}

fn main() {
    let resp = TestRequest::with_header("content-type", "text/plain")
        .run(index)
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = TestRequest::default()
        .run(index)
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
```


## Integration tests

There are several methods how you can test your application. Actix provides 
[*TestServer*](../actix_web/test/struct.TestServer.html)
server that could be used to run whole application of just specific handlers
in real http server. At the moment it is required to use third-party libraries
to make actual requests, libraries like [reqwest](https://crates.io/crates/reqwest).

In simple form *TestServer* could be configured to use handler. *TestServer::new* method
accepts configuration function, only argument for this function is *test application*
instance. You can check [api documentation](../actix_web/test/struct.TestApp.html)
for more information.

```rust
# extern crate actix_web;
extern crate reqwest;
use actix_web::*;
use actix_web::test::TestServer;

fn index(req: HttpRequest) -> HttpResponse {
     httpcodes::HTTPOk.into()
}

fn main() {
    let srv = TestServer::new(|app| app.handler(index));        // <- Start new test server
    let url = srv.url("/");                                     // <- get handler url
    assert!(reqwest::get(&url).unwrap().status().is_success()); // <- make request
}
```

Other option is to use application factory. In this case you need to pass factory function
same as you use for real http server configuration.

```rust
# extern crate actix_web;
extern crate reqwest;
use actix_web::*;
use actix_web::test::TestServer;

fn index(req: HttpRequest) -> HttpResponse {
     httpcodes::HTTPOk.into()
}

/// This function get called by http server.
fn create_app() -> Application {
    Application::new()
        .resource("/test", |r| r.h(index))
}

fn main() {
    let srv = TestServer::with_factory(create_app);             // <- Start new test server
    let url = srv.url("/test");                                 // <- get handler url
    assert!(reqwest::get(&url).unwrap().status().is_success()); // <- make request
}
```
