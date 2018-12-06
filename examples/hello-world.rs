#[macro_use]
extern crate log;
extern crate env_logger;

extern crate actix_http;
extern crate actix_net;
extern crate futures;
extern crate http;

use actix_http::{h1, Response};
use actix_net::server::Server;
use actix_net::service::NewServiceExt;
use futures::future;
use http::header::HeaderValue;
use std::env;

fn main() {
    env::set_var("RUST_LOG", "hello_world=info");
    env_logger::init();

    Server::new()
        .bind("hello-world", "127.0.0.1:8080", || {
            h1::H1Service::build()
                .client_timeout(1000)
                .client_disconnect(1000)
                .server_hostname("localhost")
                .finish(|_req| {
                    info!("{:?}", _req);
                    let mut res = Response::Ok();
                    res.header("x-head", HeaderValue::from_static("dummy value!"));
                    future::ok::<_, ()>(res.body("Hello world!"))
                })
                .map(|_| ())
        })
        .unwrap()
        .run();
}
