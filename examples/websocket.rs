//! Simple echo websocket server.
//! Open `http://localhost:8080/ws/index.html` in browser
//! or [python console client](https://github.com/actix/actix-web/blob/master/examples/websocket-client.py)
//! could be used for testing.

#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix::*;
use actix_web::*;


/// do websocket handshake and start `MyWebSocket` actor
fn ws_index(r: HttpRequest) -> Reply {
    ws::start(r, MyWebSocket).into()
}

/// websocket connection is long running connection, it easier
/// to handle with an actor
struct MyWebSocket;

impl Actor for MyWebSocket {
    type Context = HttpContext<Self>;
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
    ::std::env::set_var("RUST_LOG", "actix_web=trace");
    let _ = env_logger::init();
    let sys = actix::System::new("ws-example");

    HttpServer::new(
        || Application::new()
            // enable logger
            .middleware(middlewares::Logger::default())
            // websocket route
            .resource("/ws/", |r| r.method(Method::GET).f(ws_index))
            // static files
            .resource("/{tail:.*}",
                      |r| r.h(fs::StaticFiles::new("tail", "examples/static/", true))))
        // start http server on 127.0.0.1:8080
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
