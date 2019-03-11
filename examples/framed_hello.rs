use std::{env, io};

use actix_codec::Framed;
use actix_http::{h1, Response, SendResponse, ServiceConfig};
use actix_server::{Io, Server};
use actix_service::{fn_service, NewService};
use actix_utils::framed::IntoFramed;
use actix_utils::stream::TakeItem;
use futures::Future;

fn main() -> io::Result<()> {
    env::set_var("RUST_LOG", "framed_hello=info");
    env_logger::init();

    Server::build()
        .bind("framed_hello", "127.0.0.1:8080", || {
            fn_service(|io: Io<_>| Ok(io.into_parts().0))
                .and_then(IntoFramed::new(|| h1::Codec::new(ServiceConfig::default())))
                .and_then(TakeItem::new().map_err(|_| ()))
                .and_then(|(_req, _framed): (_, Framed<_, _>)| {
                    SendResponse::send(_framed, Response::Ok().body("Hello world!"))
                        .map_err(|_| ())
                        .map(|_| ())
                })
        })?
        .run()
}
