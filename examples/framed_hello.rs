extern crate env_logger;
extern crate log;

extern crate actix_http;
extern crate actix_net;
extern crate bytes;
extern crate futures;
extern crate http;

use actix_http::{h1, Response, SendResponse, ServiceConfig};
use actix_net::codec::Framed;
use actix_net::framed::IntoFramed;
use actix_net::server::Server;
use actix_net::service::NewServiceExt;
use actix_net::stream::TakeItem;
use futures::Future;
use std::env;

fn main() {
    env::set_var("RUST_LOG", "framed_hello=info");
    env_logger::init();

    Server::new()
        .bind("framed_hello", "127.0.0.1:8080", || {
            IntoFramed::new(|| h1::Codec::new(ServiceConfig::default()))
                .and_then(TakeItem::new().map_err(|_| ()))
                .and_then(|(_req, _framed): (_, Framed<_, _>)| {
                    SendResponse::send(_framed, Response::Ok().body("Hello world!"))
                        .map_err(|_| ())
                        .map(|_| ())
                })
        })
        .unwrap()
        .run();
}
