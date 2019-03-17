use std::{env, io};

use actix_http::{error::PayloadError, HttpService, Request, Response};
use actix_server::Server;
use bytes::BytesMut;
use futures::{Future, Stream};
use http::header::HeaderValue;
use log::info;

fn main() -> io::Result<()> {
    env::set_var("RUST_LOG", "echo=info");
    env_logger::init();

    Server::build()
        .bind("echo", "127.0.0.1:8080", || {
            HttpService::build()
                .client_timeout(1000)
                .client_disconnect(1000)
                .finish(|mut req: Request| {
                    req.take_payload()
                        .fold(BytesMut::new(), move |mut body, chunk| {
                            body.extend_from_slice(&chunk);
                            Ok::<_, PayloadError>(body)
                        })
                        .and_then(|bytes| {
                            info!("request body: {:?}", bytes);
                            let mut res = Response::Ok();
                            res.header(
                                "x-head",
                                HeaderValue::from_static("dummy value!"),
                            );
                            Ok(res.body(bytes))
                        })
                })
        })?
        .run()
}
