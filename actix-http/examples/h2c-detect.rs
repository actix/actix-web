//! An example that supports automatic selection of plaintext h1/h2c connections.
//!
//! Notably, both the following commands will work.
//! ```console
//! $ curl --http1.1 'http://localhost:8080/'
//! $ curl --http2-prior-knowledge 'http://localhost:8080/'
//! ```

use std::{convert::Infallible, io};

use actix_http::{body::BodyStream, HttpService, Request, Response, StatusCode};
use actix_server::Server;

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    Server::build()
        .bind("h2c-detect", ("127.0.0.1", 8080), || {
            HttpService::build()
                .finish(|_req: Request| async move {
                    Ok::<_, Infallible>(Response::build(StatusCode::OK).body(BodyStream::new(
                        futures_util::stream::iter([
                            Ok::<_, String>("123".into()),
                            Err("wertyuikmnbvcxdfty6t".to_owned()),
                        ]),
                    )))
                })
                .tcp_auto_h2c()
        })?
        .workers(2)
        .run()
        .await
}
