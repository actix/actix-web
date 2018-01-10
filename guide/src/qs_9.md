# WebSockets

Actix supports WebSockets out-of-the-box. It is possible to convert request's `Payload`
to a stream of [*ws::Message*](../actix_web/ws/enum.Message.html) with 
a [*ws::WsStream*](../actix_web/ws/struct.WsStream.html) and then use stream
combinators to handle actual messages. But it is simplier to handle websocket communications
with http actor.

This is example of simple websocket echo server:

```rust
# extern crate actix;
# extern crate actix_web;
use actix::*;
use actix_web::*;

/// Define http actor
struct Ws;

impl Actor for Ws {
    type Context = ws::WebsocketContext<Self>;
}

/// Define Handler for ws::Message message
impl Handler<ws::Message> for Ws {
    type Result=();

    fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context) {
        match msg {
            ws::Message::Ping(msg) => ctx.pong(&msg),
            ws::Message::Text(text) => ctx.text(&text),
            ws::Message::Binary(bin) => ctx.binary(bin),
            _ => (),
        }
    }
}

fn main() {
    Application::new()
      .resource("/ws/", |r| r.f(|req| ws::start(req, Ws)))  // <- register websocket route
      .finish();
}
```

Simple websocket echo server example is available in 
[examples directory](https://github.com/actix/actix-web/blob/master/examples/websocket.rs).

Example chat server with ability to chat over websocket connection or tcp connection
is available in [websocket-chat directory](https://github.com/actix/actix-web/tree/master/examples/websocket-chat/)
