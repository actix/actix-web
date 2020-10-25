//! This example shows how to use `actix_web::HttpServer::on_connect`

use std::any::Any;

use actix_rt::net;
use actix_web::{dev::Extensions, web, App, HttpRequest, HttpServer};

#[derive(Debug, Clone)]
struct ConnectionInfo(String);

async fn route_whoami(conn_info: web::ReqData<ConnectionInfo>) -> String {
    format!("Here is some info about you:\n{}", &conn_info.0)
}

fn on_connect(connection: &dyn Any, data: &mut Extensions) {
    let sock = connection.downcast_ref::<net::TcpStream>().unwrap();

    let msg = format!(
        "local_addr={:?}; peer_addr={:?}",
        sock.local_addr(),
        sock.peer_addr()
    );

    data.insert(ConnectionInfo(msg));
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_server=info,actix_web=info");
    env_logger::init();

    HttpServer::new(|| App::new().route("/", web::get().to(route_whoami)))
        .on_connect(on_connect)
        .bind(("127.0.0.1", 8080))?
        .workers(1)
        .run()
        .await
}
