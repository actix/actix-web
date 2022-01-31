use std::{convert::Infallible, io, time::Duration};

use actix_http::{HttpService, Request, Response, StatusCode};
use actix_server::Server;
use once_cell::sync::Lazy;

static STR: Lazy<String> = Lazy::new(|| "HELLO WORLD ".repeat(20));

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    Server::build()
        .bind("dispatcher-benchmark", ("127.0.0.1", 8080), || {
            HttpService::build()
                .client_request_timeout(Duration::from_secs(1))
                .finish(|_: Request| async move {
                    let mut res = Response::build(StatusCode::OK);
                    Ok::<_, Infallible>(res.body(&**STR))
                })
                .tcp()
        })?
        // limiting number of workers so that bench client is not sharing as many resources
        .workers(4)
        .run()
        .await
}
