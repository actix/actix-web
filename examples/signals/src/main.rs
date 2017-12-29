extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate env_logger;

use actix::*;
use actix_web::*;
use actix::actors::signal::{ProcessSignals, Subscribe};

struct MyWebSocket;

impl Actor for MyWebSocket {
    type Context = HttpContext<Self>;
}

impl StreamHandler<ws::Message> for MyWebSocket {}
impl Handler<ws::Message> for MyWebSocket {
    fn handle(&mut self, _: ws::Message, _: &mut Self::Context) -> Response<Self, ws::Message> {
        Self::empty()
    }
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("signals-example");

    let addr = HttpServer::new(|| {
        Application::new()
            // enable logger
            .middleware(middleware::Logger::default())
            .resource("/ws/", |r| r.f(|req| ws::start(req, MyWebSocket)))
            .resource("/", |r| r.h(httpcodes::HTTPOk))})
        .bind("127.0.0.1:8080").unwrap()
        .start();

    // Subscribe to unix signals
    let signals = Arbiter::system_registry().get::<ProcessSignals>();
    signals.send(Subscribe(addr.subscriber()));

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
