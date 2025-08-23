//! This example shows how to use `actix_web::HttpServer::on_connect` to access a lower-level socket
//! properties and pass them to a handler through request-local data.
//!
//! For an example of extracting a client TLS certificate, see:
//! <https://github.com/actix/examples/tree/master/https-tls/rustls-client-cert>

use std::{any::Any, io, net::SocketAddr};

use actix_web::{
    dev::Extensions, rt::net::TcpStream, web, App, HttpRequest, HttpResponse, HttpServer, Responder,
};

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ConnectionInfo {
    bind: SocketAddr,
    peer: SocketAddr,
    ttl: Option<u32>,
}

async fn route_whoami(req: HttpRequest) -> impl Responder {
    match req.conn_data::<ConnectionInfo>() {
        Some(info) => HttpResponse::Ok().body(format!(
            "Here is some info about your connection:\n\n{info:#?}",
        )),
        None => HttpResponse::InternalServerError().body("Missing expected request extension data"),
    }
}

fn get_conn_info(connection: &dyn Any, data: &mut Extensions) {
    if let Some(sock) = connection.downcast_ref::<TcpStream>() {
        data.insert(ConnectionInfo {
            bind: sock.local_addr().unwrap(),
            peer: sock.peer_addr().unwrap(),
            ttl: sock.ttl().ok(),
        });
    } else {
        unreachable!("connection should only be plaintext since no TLS is set up");
    }
}

#[actix_web::main]
async fn main() -> io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    let bind = ("127.0.0.1", 8080);
    log::info!("staring server at http://{}:{}", &bind.0, &bind.1);

    HttpServer::new(|| App::new().default_service(web::to(route_whoami)))
        .on_connect(get_conn_info)
        .bind_auto_h2c(bind)?
        .workers(2)
        .run()
        .await
}
