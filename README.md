# Actix web [![Build Status](https://travis-ci.org/actix/actix-web.svg?branch=master)](https://travis-ci.org/actix/actix-web) [![Build status](https://ci.appveyor.com/api/projects/status/kkdb4yce7qhm5w85/branch/master?svg=true)](https://ci.appveyor.com/project/fafhrd91/actix-web-hdy9d/branch/master) [![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web) [![crates.io](http://meritbadge.herokuapp.com/actix-web)](https://crates.io/crates/actix-web)

Asynchronous web framework for [Actix](https://github.com/actix/actix).

* [API Documentation (Development)](http://actix.github.io/actix-web/actix_web/)
* [API Documentation (Releases)](https://docs.rs/actix-web/)
* Cargo package: [actix-web](https://crates.io/crates/actix-web)
* Minimum supported Rust version: 1.20 or later

---

Actix web is licensed under the [Apache-2.0 license](http://opensource.org/licenses/APACHE-2.0).

## Features

  * Supported HTTP/1 and HTTP/2 protocols
  * Streaming and pipelining
  * Keep-alive and slow requests handling
  * [WebSockets](https://actix.github.io/actix-web/actix_web/ws/index.html)
  * Transparent content compression/decompression (br, gzip, deflate)
  * Configurable request routing
  * Multipart streams
  * Middlewares

## Usage

To use `actix-web`, add this to your `Cargo.toml`:

```toml
[dependencies]
actix-web = "0.2"
```

## HTTP/2

Actix web automatically upgrades connection to `http/2` if possible.

### Negotiation

`HTTP/2` protocol over tls without prior knowlage requires
[tls alpn](https://tools.ietf.org/html/rfc7301). At the moment only
`rust-openssl` supports alpn.

```toml
[dependencies]
actix-web = { git = "https://github.com/actix/actix-web", features=["alpn"] }
```

Upgrade to `http/2` schema described in
[rfc section 3.2](https://http2.github.io/http2-spec/#rfc.section.3.2) is not supported.
Starting `http/2` with prior knowledge is supported for both clear text connection
and tls connection. [rfc section 3.4](https://http2.github.io/http2-spec/#rfc.section.3.4)

[tls example](https://github.com/actix/actix-web/tree/master/examples/tls)

## Example

* [Basic](https://github.com/actix/actix-web/tree/master/examples/basic.rs)
* [Stateful](https://github.com/actix/actix-web/tree/master/examples/state.rs)
* [Mulitpart streams](https://github.com/actix/actix-web/tree/master/examples/multipart)
* [Simple websocket session](https://github.com/actix/actix-web/tree/master/examples/websocket.rs)
* [Tcp/Websocket chat](https://github.com/actix/actix-web/tree/master/examples/websocket-chat)
* [SockJS Server](https://github.com/actix/actix-sockjs)


```rust
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix::*;
use actix_web::*;


struct MyWebSocket;

/// Actor with http context
impl Actor for MyWebSocket {
    type Context = HttpContext<Self>;
}

/// Http route handler
impl Route for MyWebSocket {
    type State = ();

    fn request(req: &mut HttpRequest, ctx: &mut HttpContext<Self>) -> RouteResult<Self>
    {
        // websocket handshake
        let resp = ws::handshake(req)?;
        // send HttpResponse back to peer
        ctx.start(resp);
        // convert bytes stream to a stream of `ws::Message` and handle stream
        ctx.add_stream(ws::WsStream::new(req));
        Reply::async(MyWebSocket)
    }
}

/// Standard actix's stream handler for a stream of `ws::Message`
impl StreamHandler<ws::Message> for MyWebSocket {
    fn started(&mut self, ctx: &mut Self::Context) {
        println!("WebSocket session openned");
    }

    fn finished(&mut self, ctx: &mut Self::Context) {
        println!("WebSocket session closed");
    }
}

impl Handler<ws::Message> for MyWebSocket {
    fn handle(&mut self, msg: ws::Message, ctx: &mut HttpContext<Self>)
              -> Response<Self, ws::Message>
    {
        // process websocket messages
        println!("WS: {:?}", msg);
        match msg {
            ws::Message::Ping(msg) => ws::WsWriter::pong(ctx, &msg),
            ws::Message::Text(text) => ws::WsWriter::text(ctx, &text),
            ws::Message::Binary(bin) => ws::WsWriter::binary(ctx, bin),
            ws::Message::Closed | ws::Message::Error => {
                ctx.stop();
            }
            _ => (),
        }
        Self::empty()
    }
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("ws-example");

    HttpServer::new(
        Application::default("/")
            // enable logger
            .middleware(middlewares::Logger::default())
            // websocket route
            .resource("/ws/", |r| r.get::<MyWebSocket>())
            .route_handler("/", StaticFiles::new("examples/static/", true)))
        .serve::<_, ()>("127.0.0.1:8080").unwrap();

    Arbiter::system().send(msgs::SystemExit(0));
    let _ = sys.run();
}
```
