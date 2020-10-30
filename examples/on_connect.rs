//! This example shows how to use `actix_web::HttpServer::on_connect` to access a lower-level socket
//! properties and pass them to a handler through request-local data.
//!
//! For an example of extracting a client TLS certificate, see:
//! <https://github.com/actix/examples/tree/HEAD/rustls-client-cert>

use std::{any::Any, env, io, net::SocketAddr};

use actix_web::{dev::Extensions, rt::net::TcpStream, web, App, HttpServer};

#[derive(Debug, Clone)]
struct ConnectionInfo {
    bind: SocketAddr,
    peer: SocketAddr,
    ttl: Option<u32>,
}

async fn route_whoami(conn_info: web::ReqData<ConnectionInfo>) -> String {
    format!(
        "Here is some info about your connection:\n\n{:#?}",
        conn_info
    )
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
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }

    env_logger::init();

    HttpServer::new(|| App::new().default_service(web::to(route_whoami)))
        .on_connect(get_conn_info)
        .bind(("127.0.0.1", 8080))?
        .workers(1)
        .run()
        .await
}
