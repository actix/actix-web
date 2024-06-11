use std::{convert::Infallible, io, time::Duration};

use actix_http::{header::HeaderValue, HttpService, Request, Response, StatusCode};
use actix_server::Server;
use tracing::info;

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    Server::build()
        .bind("hello-world", ("127.0.0.1", 8080), || {
            HttpService::build()
                .client_request_timeout(Duration::from_secs(1))
                .client_disconnect_timeout(Duration::from_secs(1))
                .on_connect_ext(|_, ext| {
                    ext.insert(42u32);
                })
                .finish(|req: Request| async move {
                    info!("{:?}", req);

                    let mut res = Response::build(StatusCode::OK);
                    res.insert_header(("x-head", HeaderValue::from_static("dummy value!")));

                    let forty_two = req.conn_data::<u32>().unwrap().to_string();
                    res.insert_header(("x-forty-two", HeaderValue::from_str(&forty_two).unwrap()));

                    Ok::<_, Infallible>(res.body("Hello world!"))
                })
                .tcp()
        })?
        .run()
        .await
}
