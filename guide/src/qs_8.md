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
in real http server. *TrstServer::get()*, *TrstServer::post()* or *TrstServer::client()*
methods could be used to send request to test server.

In simple form *TestServer* could be configured to use handler. *TestServer::new* method
accepts configuration function, only argument for this function is *test application*
instance. You can check [api documentation](../actix_web/test/struct.TestApp.html)
for more information.

```rust
# extern crate actix_web;
use actix_web::*;
use actix_web::test::TestServer;

fn index(req: HttpRequest) -> HttpResponse {
     httpcodes::HTTPOk.into()
}

fn main() {
    let mut srv = TestServer::new(|app| app.handler(index));  // <- Start new test server
    
    let request = srv.get().finish().unwrap();                // <- create client request
    let response = srv.execute(request.send()).unwrap();      // <- send request to the server
    assert!(response.status().is_success());                  // <- check response
    
    let bytes = srv.execute(response.body()).unwrap();        // <- read response body
}
```

Other option is to use application factory. In this case you need to pass factory function
same as you use for real http server configuration.

```rust
# extern crate http;
# extern crate actix_web;
use http::Method;
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
    let mut srv = TestServer::with_factory(create_app);         // <- Start new test server

    let request = srv.client(Method::GET, "/test").finish().unwrap(); // <- create client request
    let response = srv.execute(request.send()).unwrap();        // <- send request to the server

    assert!(response.status().is_success());                    // <- check response
}
```

## WebSocket server tests

It is possible to register *handler* with `TestApp::handler()` method that
initiate web socket connection. *TestServer* provides `ws()` which connects to
websocket server and returns ws reader and writer objects. *TestServer* also 
provides `execute()` method which runs future object to completion and returns
result of the future computation.

Here is simple example, that shows how to test server websocket handler.

```rust
# extern crate actix;
# extern crate actix_web;
# extern crate futures;
# extern crate http;
# extern crate bytes;

use actix_web::*;
use futures::Stream;
# use actix::prelude::*;

struct Ws;   // <- WebSocket actor

impl Actor for Ws {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<ws::Message, ws::WsError> for Ws {

    fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context) {
        match msg {
            ws::Message::Text(text) => ctx.text(text),
            _ => (),
        }
    }
}

fn main() {
    let mut srv = test::TestServer::new(             // <- start our server with ws handler
        |app| app.handler(|req| ws::start(req, Ws)));

    let (reader, mut writer) = srv.ws().unwrap();    // <- connect to ws server

    writer.text("text");                             // <- send message to server
    
    let (item, reader) = srv.execute(reader.into_future()).unwrap();  // <- wait for one message
    assert_eq!(item, Some(ws::Message::Text("text".to_owned())));
}
```
