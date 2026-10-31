use std::net::TcpListener;

use actix_http::HttpService;
use actix_server::Server;
use actix_service::map_config;
use actix_web::{dev::AppConfig, get, App, Responder};

#[get("/")]
async fn index() -> impl Responder {
    "Hello, world. From Actix Web!"
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080")?;
    let addr = listener.local_addr()?;

    // Register additional listeners on the same server builder, each with its own AppConfig.
    Server::build()
        .listen("hello-world", listener, move || {
            // construct actix-web app
            let app = App::new().service(index);

            HttpService::build()
                .local_addr(addr)
                // pass the app to service builder
                // map_config is used to map App's configuration to ServiceBuilder
                // h1 will configure server to only use HTTP/1.1
                .h1(map_config(app, move |_| {
                    AppConfig::new(false, addr.to_string(), addr)
                }))
                .tcp()
        })?
        .run()
        .await
}
