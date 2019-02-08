use actix_http::http::HeaderValue;
use actix_http::HttpMessage;
use actix_http::{h1, Error, Request, Response};
use actix_server::Server;
use actix_service::NewService;
use bytes::Bytes;
use futures::Future;
use log::info;
use std::env;

fn handle_request(req: Request) -> impl Future<Item = Response, Error = Error> {
    req.body().limit(512).from_err().and_then(|bytes: Bytes| {
        info!("request body: {:?}", bytes);
        let mut res = Response::Ok();
        res.header("x-head", HeaderValue::from_static("dummy value!"));
        Ok(res.body(bytes))
    })
}

fn main() {
    env::set_var("RUST_LOG", "echo=info");
    env_logger::init();

    Server::build()
        .bind("echo", "127.0.0.1:8080", || {
            h1::H1Service::build()
                .client_timeout(1000)
                .client_disconnect(1000)
                .server_hostname("localhost")
                .finish(|_req: Request| handle_request(_req))
                .map(|_| ())
        })
        .unwrap()
        .run();
}
