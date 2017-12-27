# WebSockets

Actix supports WebSockets out-of-the-box. It is possible to convert request's `Payload`
to a stream of [*ws::Message*](../actix_web/ws/enum.Message.html) with 
a [*ws::WsStream*](../actix_web/ws/struct.WsStream.html) and then use stream
combinators to handle actual messages. But it is simplier to handle websocket communications
with http actor.

```rust
extern crate actix;
extern crate actix_web;

use actix::*;
use actix_web::*;

/// Define http actor
struct Ws;

impl Actor for Ws {
    type Context = HttpContext<Self>;
}

/// Define Handler for ws::Message message
# impl StreamHandler<ws::Message> for WsRoute {}
impl Handler<ws::Message> for WsRoute {
    fn handle(&mut self, msg: ws::Message, ctx: &mut HttpContext<Self>) -> Response<Self, ws::Message>
    {
        match msg {
            ws::Message::Ping(msg) => ws::WsWriter::pong(ctx, &msg),
            ws::Message::Text(text) => ws::WsWriter::text(ctx, &text),
            ws::Message::Binary(bin) => ws::WsWriter::binary(ctx, bin),
            _ => (),
        }
        Self::empty()
    }
}

fn main() {
    Application::new()
      .resource("/ws/", |r| r.f(|req| ws::start(req, WS))  // <- register websocket route
      .finish();
}
```

Simple websocket echo server example is available in 
[examples directory](https://github.com/actix/actix-web/blob/master/examples/websocket.rs).

Example chat server with ability to chat over websocket connection or tcp connection
is available in [websocket-chat directory](https://github.com/actix/actix-web/tree/master/examples/websocket-chat/)
