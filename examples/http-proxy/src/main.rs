extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate env_logger;

use actix_web::*;
use futures::{Future, Stream};


/// Stream client request response and then send body to a server response
fn index(_req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    client::ClientRequest::get("https://www.rust-lang.org/en-US/")
        .finish().unwrap()
        .send()
        .map_err(error::Error::from)   // <- convert SendRequestError to an Error
        .and_then(
            |resp| resp.body()         // <- this is MessageBody type, resolves to complete body
                .from_err()            // <- convet PayloadError to a Error
                .and_then(|body| {     // <- we got complete body, now send as server response
                    httpcodes::HttpOk.build()
                        .body(body)
                        .map_err(error::Error::from)
                }))
        .responder()
}

/// stream client request to a server response
fn streaming(_req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    // send client request
    client::ClientRequest::get("https://www.rust-lang.org/en-US/")
        .finish().unwrap()
        .send()                         // <- connect to host and send request
        .map_err(error::Error::from)    // <- convert SendRequestError to an Error
        .and_then(|resp| {              // <- we received client response
            httpcodes::HttpOk.build()
                // read one chunk from client response and send this chunk to a server response
                // .from_err() converts PayloadError to a Error
                .body(Body::Streaming(Box::new(resp.from_err())))
                .map_err(|e| e.into()) // HttpOk::build() mayb return HttpError, we need to convert it to a Error
        })
        .responder()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();
    let sys = actix::System::new("http-proxy");

    let _addr = HttpServer::new(
        || Application::new()
            .middleware(middleware::Logger::default())
            .resource("/streaming", |r| r.f(streaming))
            .resource("/", |r| r.f(index)))
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
