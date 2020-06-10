use std::{env, io};

use actix_http::http::HeaderValue;
use actix_http::{Error, HttpService, Request, Response};
use actix_server::Server;
use bytes::BytesMut;
use futures_util::StreamExt;
use log::info;

async fn handle_request(mut req: Request) -> Result<Response, Error> {
    let mut body = BytesMut::new();
    while let Some(item) = req.payload().next().await {
        body.extend_from_slice(&item?)
    }

    info!("request body: {:?}", body);
    Ok(Response::Ok()
        .header("x-head", HeaderValue::from_static("dummy value!"))
        .body(body))
}

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env::set_var("RUST_LOG", "echo=info");
    env_logger::init();

    Server::build()
        .bind("echo", "127.0.0.1:8080", || {
            HttpService::build().finish(handle_request).tcp()
        })?
        .run()
        .await
}
