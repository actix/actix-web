#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;
extern crate openssl;

use openssl::ssl::{SslMethod, SslAcceptor, SslFiletype};
use actix_web::{
    http, middleware, server, App, HttpRequest, HttpResponse, Error};


/// simple handle
fn index(req: HttpRequest) -> Result<HttpResponse, Error> {
    println!("{:?}", req);
    Ok(HttpResponse::Ok()
       .content_type("text/plain")
       .body("Welcome!"))
}

fn main() {
    if ::std::env::var("RUST_LOG").is_err() {
        ::std::env::set_var("RUST_LOG", "actix_web=info");
    }
    env_logger::init();
    let sys = actix::System::new("ws-example");

    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder.set_private_key_file("key.pem", SslFiletype::PEM).unwrap();
    builder.set_certificate_chain_file("cert.pem").unwrap();

    server::new(
        || App::new()
            // enable logger
            .middleware(middleware::Logger::default())
            // register simple handler, handle all methods
            .resource("/index.html", |r| r.f(index))
            // with path parameters
            .resource("/", |r| r.method(http::Method::GET).f(|req| {
                HttpResponse::Found()
                    .header("LOCATION", "/index.html")
                    .finish()
            })))
        .bind("127.0.0.1:8443").unwrap()
        .start_ssl(builder).unwrap();

    println!("Started http server: 127.0.0.1:8443");
    let _ = sys.run();
}
