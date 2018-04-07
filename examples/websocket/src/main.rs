//! Simple echo websocket server.
//! Open `http://localhost:8080/ws/index.html` in browser
//! or [python console client](https://github.com/actix/actix-web/blob/master/examples/websocket-client.py)
//! could be used for testing.

#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix::prelude::*;
use actix_web::{
    http, middleware, server, fs, ws, App, HttpRequest, HttpResponse, Error};

/// do websocket handshake and start `MyWebSocket` actor
fn ws_index(r: HttpRequest) -> Result<HttpResponse, Error> {
    ws::start(r, MyWebSocket)
}

/// websocket connection is long running connection, it easier
/// to handle with an actor
struct MyWebSocket;

impl Actor for MyWebSocket {
    type Context = ws::WebsocketContext<Self>;
}

/// Handler for `ws::Message`
impl StreamHandler<ws::Message, ws::ProtocolError> for MyWebSocket {

    fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context) {
        // process websocket messages
        println!("WS: {:?}", msg);
        match msg {
            ws::Message::Ping(msg) => ctx.pong(&msg),
            ws::Message::Text(text) => ctx.text(text),
            ws::Message::Binary(bin) => ctx.binary(bin),
            ws::Message::Close(_) => {
                ctx.stop();
            }
            _ => (),
        }
    }
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();
    let sys = actix::System::new("ws-example");

    server::new(
        || App::new()
            // enable logger
            .middleware(middleware::Logger::default())
            // websocket route
            .resource("/ws/", |r| r.method(http::Method::GET).f(ws_index))
            // static files
            .handler("/", fs::StaticFiles::new("../static/")
                     .index_file("index.html")))
        // start http server on 127.0.0.1:8080
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
