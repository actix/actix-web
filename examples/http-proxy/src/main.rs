extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate env_logger;

use actix_web::*;
use futures::Future;
use futures::future::{ok, err, Either};


fn index(_req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    client::ClientRequest::get("https://www.rust-lang.org/en-US/")
        .finish().unwrap()
        .send()
        .map_err(|e| error::Error::from(error::ErrorInternalServerError(e)))
        .then(|result| match result {
            Ok(resp) => {
                Either::A(resp.body().from_err().and_then(|body| {
                    match httpcodes::HttpOk.build().body(body) {
                        Ok(resp) => ok(resp),
                        Err(e) => err(e.into()),
                    }
                }))
            },
            Err(e) => {
                Either::B(err(error::Error::from(e)))
            }
        })
        .responder()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("ws-example");

    let _addr = HttpServer::new(
        || Application::new()
            // enable logger
            .middleware(middleware::Logger::default())
            .resource("/", |r| r.f(index)))
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
