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
                .client_request_timeout(Duration::from_secs(20))
                .client_disconnect_timeout(Duration::from_secs(20))
                .finish(|mut req: Request| async move {
                    let mut body = BytesMut::new();
                    while let Some(item) = req.payload().next().await {
                        body.extend_from_slice(&item?);
                    }

                    info!("request body: {body:?}");

                    let res = Response::build(StatusCode::OK)
                        .insert_header(("x-head", HeaderValue::from_static("dummy value!")))
                        .body(body);

                    Ok::<_, Error>(res)
                })
                .tcp_auto_h2c_proxy_protocol_v1()
        })?
        .workers(2)
        .run()
        .await
}

static_assertions::assert_impl_all!(
    tokio::io::BufReader<tokio::net::TcpStream>:
    tokio::io::AsyncRead,
    tokio::io::AsyncWrite,
    Unpin,
);
