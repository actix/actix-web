extern crate actix;
extern crate actix_net;
extern crate actix_web;

use std::{thread, time};

use actix::System;
use actix_net::server::Server;
use actix_net::service::NewServiceExt;
use actix_web::server::{HttpService, KeepAlive, ServiceConfig, StreamConfiguration};
use actix_web::{client, http, test, App, HttpRequest};

#[test]
fn test_custom_pipeline() {
    let addr = test::TestServer::unused_addr();

    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                let app = App::new()
                    .route("/", http::Method::GET, |_: HttpRequest| "OK")
                    .finish();
                let settings = ServiceConfig::build(app)
                    .keep_alive(KeepAlive::Disabled)
                    .client_timeout(1000)
                    .client_shutdown(1000)
                    .server_hostname("localhost")
                    .server_address(addr)
                    .finish();

                StreamConfiguration::new()
                    .nodelay(true)
                    .tcp_keepalive(Some(time::Duration::from_secs(10)))
                    .and_then(HttpService::new(settings))
            }).unwrap()
            .run();
    });

    let mut sys = System::new("test");
    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str())
            .finish()
            .unwrap();
        let response = sys.block_on(req.send()).unwrap();
        assert!(response.status().is_success());
    }
}

#[test]
fn test_h1() {
    use actix_web::server::H1Service;

    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                let app = App::new()
                    .route("/", http::Method::GET, |_: HttpRequest| "OK")
                    .finish();
                let settings = ServiceConfig::build(app)
                    .keep_alive(KeepAlive::Disabled)
                    .client_timeout(1000)
                    .client_shutdown(1000)
                    .server_hostname("localhost")
                    .server_address(addr)
                    .finish();

                H1Service::new(settings)
            }).unwrap()
            .run();
    });

    let mut sys = System::new("test");
    {
        let req = client::ClientRequest::get(format!("http://{}/", addr).as_str())
            .finish()
            .unwrap();
        let response = sys.block_on(req.send()).unwrap();
        assert!(response.status().is_success());
    }
}
