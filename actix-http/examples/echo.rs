use std::{io, time::Duration};

use actix_http::{Error, HttpService, Request, Response, StatusCode};
use actix_server::Server;
use bytes::BytesMut;
use futures_util::StreamExt as _;
use http::header::HeaderValue;
use tracing::info;

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    Server::build()
        .bind("echo", ("127.0.0.1", 8080), || {
            HttpService::build()
                .client_request_timeout(Duration::from_secs(1))
                .client_disconnect_timeout(Duration::from_secs(1))
                // handles HTTP/1.1 and HTTP/2
                .finish(|mut req: Request| async move {
                    let mut body = BytesMut::new();
                    while let Some(item) = req.payload().next().await {
                        body.extend_from_slice(&item?);
                    }

                    info!("request body: {:?}", body);

                    let res = Response::build(StatusCode::OK)
                        .insert_header(("x-head", HeaderValue::from_static("dummy value!")))
                        .body(body);

                    Ok::<_, Error>(res)
                })
                // No TLS
                .tcp()
        })?
        .run()
        .await
}
