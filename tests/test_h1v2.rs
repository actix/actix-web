extern crate actix;
extern crate actix_http;
extern crate actix_net;
extern crate actix_web;
extern crate futures;

use std::thread;

use actix::System;
use actix_net::server::Server;
use actix_web::{client, test};
use futures::future;

use actix_http::server::KeepAlive;
use actix_http::{h1, Error, HttpResponse, ServiceConfig};

#[test]
fn test_h1_v2() {
    let addr = test::TestServer::unused_addr();
    thread::spawn(move || {
        Server::new()
            .bind("test", addr, move || {
                let settings = ServiceConfig::build()
                    .keep_alive(KeepAlive::Disabled)
                    .client_timeout(1000)
                    .client_shutdown(1000)
                    .server_hostname("localhost")
                    .server_address(addr)
                    .finish();

                h1::H1Service::new(settings, |req| {
                    println!("REQ: {:?}", req);
                    future::ok::<_, Error>(HttpResponse::Ok().finish())
                })
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
