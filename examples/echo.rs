use actix_http::HttpMessage;
use actix_http::{h1, Request, Response};
use actix_server::Server;
use actix_service::NewService;
use bytes::Bytes;
use futures::Future;
use http::header::HeaderValue;
use log::info;
use std::env;

fn main() {
    env::set_var("RUST_LOG", "echo=info");
    env_logger::init();

    Server::build()
        .bind("echo", "127.0.0.1:8080", || {
            h1::H1Service::build()
                .client_timeout(1000)
                .client_disconnect(1000)
                .server_hostname("localhost")
                .finish(|mut req: Request| {
                    req.body().limit(512).and_then(|bytes: Bytes| {
                        info!("request body: {:?}", bytes);
                        let mut res = Response::Ok();
                        res.header("x-head", HeaderValue::from_static("dummy value!"));
                        Ok(res.body(bytes))
                    })
                })
                .map(|_| ())
        })
        .unwrap()
        .run();
}
