//! This example shows how to use `actix_web::HttpServer::on_connect` to access a lower-level socket
//! properties and pass them to a handler through request-local data.
//!
//! For an example of extracting a client TLS certificate, see:
//! <https://github.com/actix/examples/tree/HEAD/rustls-client-cert>

use std::{any::Any, env, fs::File, io::BufReader};

use actix_tls::rustls::{ServerConfig, TlsStream};
use actix_web::{
    dev::Extensions, rt::net::TcpStream, web, App, HttpResponse, HttpServer, Responder,
};
use log::info;
use rust_tls::{
    internal::pemfile::{certs, pkcs8_private_keys},
    AllowAnyAnonymousOrAuthenticatedClient, Certificate, RootCertStore, Session,
};

const CA_CERT: &str = "examples/certs/rootCA.pem";
const SERVER_CERT: &str = "examples/certs/server-cert.pem";
const SERVER_KEY: &str = "examples/certs/server-key.pem";

#[derive(Debug, Clone)]
struct ConnectionInfo(String);

async fn route_whoami(
    conn_info: web::ReqData<ConnectionInfo>,
    client_cert: Option<web::ReqData<Certificate>>,
) -> impl Responder {
    if let Some(cert) = client_cert {
        HttpResponse::Ok().body(format!("{:?}\n\n{:?}", &conn_info, &cert))
    } else {
        HttpResponse::Unauthorized().body("No client certificate provided.")
    }
}

fn get_client_cert(connection: &dyn Any, data: &mut Extensions) {
    if let Some(tls_socket) = connection.downcast_ref::<TlsStream<TcpStream>>() {
        info!("TLS on_connect");

        let (socket, tls_session) = tls_socket.get_ref();

        let msg = format!(
            "local_addr={:?}; peer_addr={:?}",
            socket.local_addr(),
            socket.peer_addr()
        );

        data.insert(ConnectionInfo(msg));

        if let Some(mut certs) = tls_session.get_peer_certificates() {
            info!("client certificate found");

            // insert a `rustls::Certificate` into request data
            data.insert(certs.pop().unwrap());
        }
    } else if let Some(socket) = connection.downcast_ref::<TcpStream>() {
        info!("plaintext on_connect");

        let msg = format!(
            "local_addr={:?}; peer_addr={:?}",
            socket.local_addr(),
            socket.peer_addr()
        );

        data.insert(ConnectionInfo(msg));
    } else {
        unreachable!("socket should be TLS or plaintext");
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }

    env_logger::init();

    let ca_cert = &mut BufReader::new(File::open(CA_CERT)?);

    let mut cert_store = RootCertStore::empty();
    cert_store
        .add_pem_file(ca_cert)
        .expect("root CA not added to store");
    let client_auth = AllowAnyAnonymousOrAuthenticatedClient::new(cert_store);

    let mut config = ServerConfig::new(client_auth);

    let cert_file = &mut BufReader::new(File::open(SERVER_CERT)?);
    let key_file = &mut BufReader::new(File::open(SERVER_KEY)?);

    let cert_chain = certs(cert_file).unwrap();
    let mut keys = pkcs8_private_keys(key_file).unwrap();
    config.set_single_cert(cert_chain, keys.remove(0)).unwrap();

    HttpServer::new(|| App::new().route("/", web::get().to(route_whoami)))
        .on_connect(get_client_cert)
        .bind(("localhost", 8080))?
        .bind_rustls(("localhost", 8443), config)?
        .workers(1)
        .run()
        .await
}
