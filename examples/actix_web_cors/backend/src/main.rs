#![allow(warnings)]

#[macro_use] extern crate serde_derive;
extern crate serde;
extern crate serde_json;
extern crate futures;
extern crate num_cpus;
extern crate actix;
extern crate actix_web;
extern crate env_logger;
extern crate http;

use actix::*;
use actix_web::*;
use std::env;

mod user;

use http::header;
use actix_web::middleware::cors;
use user:: info;

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("Actix-web-CORS");
    env::set_var("RUST_BACKTRACE", "1");
    HttpServer::new(
        || Application::new()
            .middleware(middleware::Logger::default())
            .resource("/user/info", |r| {
                cors::Cors::build()
                .allowed_origin("http://localhost:1234")
                .allowed_methods(vec!["GET", "POST"])
                .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
                .allowed_header(header::CONTENT_TYPE)
                .max_age(3600)
                .finish().expect("Can not create CORS middleware")
                .register(r); 
                r.method(Method::POST).a(info);
            }))
        .bind("127.0.0.1:8000").unwrap()
        .shutdown_timeout(200)
        .start();

    let _ = sys.run();
}