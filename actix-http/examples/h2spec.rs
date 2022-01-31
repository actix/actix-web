use std::{convert::Infallible, io};

use actix_http::{HttpService, Request, Response, StatusCode};
use actix_server::Server;
use once_cell::sync::Lazy;

static STR: Lazy<String> = Lazy::new(|| "HELLO WORLD ".repeat(100));

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    Server::build()
        .bind("h2spec", ("127.0.0.1", 8080), || {
            HttpService::build()
                .h2(|_: Request| async move {
                    let mut res = Response::build(StatusCode::OK);
                    Ok::<_, Infallible>(res.body(&**STR))
                })
                .tcp()
        })?
        .workers(4)
        .run()
        .await
}
