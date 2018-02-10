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
fn ws_index(r: HttpRequest) -> Result<HttpResponse> {
    ws::start(r, MyWebSocket)
}

/// websocket connection is long running connection, it easier
/// to handle with an actor
struct MyWebSocket;

impl Actor for MyWebSocket {
    type Context = ws::WebsocketContext<Self>;
}

/// Handler for `ws::Message`
impl Handler<ws::Message> for MyWebSocket {
    type Result = ();

    fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context) {
        // process websocket messages
        println!("WS: {:?}", msg);
        match msg {
            ws::Message::Ping(msg) => ctx.pong(&msg),
            ws::Message::Text(text) => ctx.text(text),
            ws::Message::Binary(bin) => ctx.binary(bin),
            ws::Message::Closed | ws::Message::Error => {
                ctx.stop();
            }
            _ => (),
        }
    }
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("ws-example");

    let _addr = HttpServer::new(
        || Application::new()
            // enable logger
            .middleware(middleware::Logger::default())
            // websocket route
            .resource("/ws/", |r| r.method(Method::GET).f(ws_index))
            // static files
            .handler("/", fs::StaticFiles::new("../static/", true)
                     .index_file("index.html")))
        // start http server on 127.0.0.1:8080
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
