#[macro_use] extern crate serde_derive;
extern crate serde;
extern crate serde_json;
extern crate futures;
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use std::env;
use actix_web::{http, middleware, server, App};

mod user;
use user::info;


fn main() {
    env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();

    let sys = actix::System::new("Actix-web-CORS");

    server::new(
        || App::new()
            .middleware(middleware::Logger::default())
            .resource("/user/info", |r| {
                middleware::cors::Cors::build()
                    .allowed_origin("http://localhost:1234")
                    .allowed_methods(vec!["GET", "POST"])
                    .allowed_headers(
                        vec![http::header::AUTHORIZATION,
                             http::header::ACCEPT,
                             http::header::CONTENT_TYPE])
                    .max_age(3600)
                    .finish().expect("Can not create CORS middleware")
                    .register(r);
                r.method(http::Method::POST).a(info);
            }))
        .bind("127.0.0.1:8000").unwrap()
        .shutdown_timeout(200)
        .start();

    let _ = sys.run();
}
