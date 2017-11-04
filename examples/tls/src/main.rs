#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;

//use tokio_tls;
use std::fs::File;
use std::io::Read;
// use native_tls::{TlsAcceptor, TlsStream};

use actix_web::*;

/// somple handle
fn index(req: &mut HttpRequest, _payload: Payload, state: &()) -> HttpResponse {
    println!("{:?}", req);
    httpcodes::HTTPOk.with_body("Welcome!")
}

fn main() {
    if ::std::env::var("RUST_LOG").is_err() {
        ::std::env::set_var("RUST_LOG", "actix_web=info");
    }
    let _ = env_logger::init();
    let sys = actix::System::new("ws-example");

    let mut file = File::open("identity.pfx").unwrap();
    let mut pkcs12 = vec![];
    file.read_to_end(&mut pkcs12).unwrap();
    let pkcs12 = Pkcs12::from_der(&pkcs12, "12345").unwrap();

    HttpServer::new(
        Application::default("/")
            // enable logger
            .middleware(Logger::new(None))
            // register simple handler, handle all methods
            .handler("/index.html", index)
            // with path parameters
            .resource("/", |r| r.handler(Method::GET, |req, _, _| {
                Ok(httpcodes::HTTPFound
                   .builder()
                   .header("LOCATION", "/index.html")
                   .body(Body::Empty)?)
            })))
        .serve_tls::<_, ()>("127.0.0.1:8080", pkcs12).unwrap();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
