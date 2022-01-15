use std::{convert::Infallible, io};

use actix_http::{body::EitherBody, HttpService, Request, Response, StatusCode};
use actix_server::Server;

#[actix_rt::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    Server::build()
        .bind("hello-world", ("127.0.0.1", 8080), || {
            HttpService::build()
                .client_timeout(1000)
                .client_disconnect(1000)
                .finish(|req: Request| async move {
                    let mut res = Response::build(StatusCode::OK).body("Hello world!");

                    match req.path() {
                        "/" => {}

                        "/ext" => {
                            res.extensions_mut().insert(123u8);
                        }

                        "/more" => {
                            res.extensions_mut().insert(123u8);
                            res.extensions_mut().insert(123u16);
                            res.extensions_mut().insert(123u32);
                            res.extensions_mut().insert(123u64);
                            res.extensions_mut().insert(123u128);
                            res.extensions_mut().insert(123i8);
                            res.extensions_mut().insert(123i16);
                            res.extensions_mut().insert(123i32);
                            res.extensions_mut().insert(123i64);
                            res.extensions_mut().insert(123i128);
                        }

                        _ => {
                            return Ok(Response::not_found()
                                .map_body(|_, body| EitherBody::right(body)))
                        }
                    }

                    Ok::<_, Infallible>(res)
                })
                .tcp()
        })?
        .workers(4)
        .run()
        .await
}
