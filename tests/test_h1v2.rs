extern crate actix;
extern crate actix_net;
extern crate actix_web;
extern crate futures;

use std::thread;

use actix::System;
use actix_net::server::Server;
use actix_net::service::{IntoNewService, IntoService};
use futures::future;

use actix_web::server::h1disp::Http1Dispatcher;
use actix_web::server::KeepAlive;
use actix_web::server::ServiceConfig;
use actix_web::{client, test, App, Error, HttpRequest, HttpResponse};

#[test]
fn test_h1_v2() {
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

                (move |io| {
                    let pool = settings.request_pool();
                    Http1Dispatcher::new(
                        io,
                        pool,
                        (|req| {
                            println!("REQ: {:?}", req);
                            future::ok::<_, Error>(HttpResponse::Ok().finish())
                        }).into_service(),
                    )
                }).into_new_service()
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
