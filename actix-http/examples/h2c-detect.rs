//! An example that supports automatic selection of plaintext h1/h2c connections.
//!
//! Notably, both the following commands will work.
//! ```console
//! $ curl --http1.1 'http://localhost:8080/'
//! $ curl --http2-prior-knowledge 'http://localhost:8080/'
//! ```

use std::{convert::Infallible, io};

use actix_http::{HttpService, Protocol, Request, Response, StatusCode};
use actix_rt::net::TcpStream;
use actix_server::Server;
use actix_service::{fn_service, ServiceFactoryExt};

const H2_PREFACE: &[u8] = b"PRI * HTTP/2";

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    Server::build()
        .bind("h2c-detect", ("127.0.0.1", 8080), || {
            fn_service(move |io: TcpStream| async move {
                let mut buf = [0; 12];

                io.peek(&mut buf).await?;

                let proto = if buf == H2_PREFACE {
                    tracing::info!("selecting h2c");
                    Protocol::Http2
                } else {
                    tracing::info!("selecting h1");
                    Protocol::Http1
                };

                let peer_addr = io.peer_addr().ok();
                Ok::<_, io::Error>((io, proto, peer_addr))
            })
            .and_then(
                HttpService::build()
                    .finish(|_req: Request| async move {
                        Ok::<_, Infallible>(Response::build(StatusCode::OK).body("Hello!"))
                    })
                    .map_err(|err| {
                        tracing::error!("{}", err);
                        io::Error::new(io::ErrorKind::Other, "http service dispatch error")
                    }),
            )
        })?
        .workers(2)
        .run()
        .await
}
