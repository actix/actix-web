//! This example shows how to use `actix_web::HttpServer::on_connect`

#[derive(Clone)]
struct ConnectionInfo(String);

async fn route_whoami(req: actix_web::HttpRequest) -> String {
    let extensions = req.extensions();
    let conn_info = extensions.get::<ConnectionInfo>().unwrap();
    format!("Here is some info about you: {}", conn_info.0)
}

fn on_connect(connection: &dyn std::any::Any) -> ConnectionInfo {
    let sock = connection.downcast_ref::<actix_rt::net::TcpStream>().unwrap();
    let msg = format!("local_addr={:?}\npeer_addr={:?}", sock.local_addr(),sock.peer_addr());
    ConnectionInfo(msg)
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_server=info,actix_web=info");
    env_logger::init();

    actix_web::HttpServer::new(|| {
        actix_web::App::new().route("/", actix_web::web::get().to(route_whoami))
    })
    .on_connect(std::sync::Arc::new(on_connect))
    .bind("127.0.0.1:8080")?
    .workers(1)
    .run()
    .await
}
