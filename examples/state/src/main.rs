#![cfg_attr(feature="cargo-clippy", allow(needless_pass_by_value))]
//! There are two level of statefulness in actix-web. Application has state
//! that is shared across all handlers within same Application.
//! And individual handler can have state.

extern crate actix;
extern crate actix_web;
extern crate env_logger;

use std::cell::Cell;

use actix::prelude::*;
use actix_web::{
    http, server, ws, middleware, App, HttpRequest, HttpResponse};

/// Application state
struct AppState {
    counter: Cell<usize>,
}

/// simple handle
fn index(req: HttpRequest<AppState>) -> HttpResponse {
    println!("{:?}", req);
    req.state().counter.set(req.state().counter.get() + 1);

    HttpResponse::Ok().body(format!("Num of requests: {}", req.state().counter.get()))
}

/// `MyWebSocket` counts how many messages it receives from peer,
/// websocket-client.py could be used for tests
struct MyWebSocket {
    counter: usize,
}

impl Actor for MyWebSocket {
    type Context = ws::WebsocketContext<Self, AppState>;
}

impl StreamHandler<ws::Message, ws::ProtocolError> for MyWebSocket {

    fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context) {
        self.counter += 1;
        println!("WS({}): {:?}", self.counter, msg);
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
        || App::with_state(AppState{counter: Cell::new(0)})
            // enable logger
            .middleware(middleware::Logger::default())
            // websocket route
            .resource(
                "/ws/", |r|
                r.method(http::Method::GET).f(
                    |req| ws::start(req, MyWebSocket{counter: 0})))
            // register simple handler, handle all methods
            .resource("/", |r| r.f(index)))
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
