use std::io;

use actix_http::{http::StatusCode, HttpService, Response};
use actix_server::Server;
use actix_utils::future;
use http::header::HeaderValue;
use log::info;

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    Server::build()
        .bind("hello-world", "127.0.0.1:8080", || {
            HttpService::build()
                .client_timeout(1000)
                .client_disconnect(1000)
                .finish(|_req| {
                    info!("{:?}", _req);
                    let mut res = Response::build(StatusCode::OK);
                    res.insert_header((
                        "x-head",
                        HeaderValue::from_static("dummy value!"),
                    ));
                    future::ok::<_, ()>(res.body("Hello world!"))
                })
                .tcp()
        })?
        .run()
        .await
}
