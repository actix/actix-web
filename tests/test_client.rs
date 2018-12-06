extern crate actix;
extern crate actix_http;
extern crate actix_net;
extern crate bytes;
extern crate futures;

use actix_net::service::NewServiceExt;
use bytes::Bytes;
use futures::future::{self, ok};

use actix_http::HttpMessage;
use actix_http::{client, h1, test::TestServer, Request, Response};

const STR: &str = "Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World";

#[test]
fn test_h1_v2() {
    let mut srv = TestServer::with_factory(move || {
        h1::H1Service::build()
            .finish(|_| future::ok::<_, ()>(Response::Ok().body(STR)))
            .map(|_| ())
    });
    let mut connector = srv.new_connector();

    let request = srv.get().finish().unwrap();
    let response = srv.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    let request = srv.get().header("x-test", "111").finish().unwrap();
    let repr = format!("{:?}", request);
    assert!(repr.contains("ClientRequest"));
    assert!(repr.contains("x-test"));

    let response = srv.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    let request = srv.post().finish().unwrap();
    let response = srv.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_connection_close() {
    let mut srv = TestServer::with_factory(move || {
        h1::H1Service::build()
            .finish(|_| ok::<_, ()>(Response::Ok().body(STR)))
            .map(|_| ())
    });
    let mut connector = srv.new_connector();

    let request = srv.get().close().finish().unwrap();
    let response = srv.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_with_query_parameter() {
    let mut srv = TestServer::with_factory(move || {
        h1::H1Service::build()
            .finish(|req: Request| {
                if req.uri().query().unwrap().contains("qp=") {
                    ok::<_, ()>(Response::Ok().finish())
                } else {
                    ok::<_, ()>(Response::BadRequest().finish())
                }
            })
            .map(|_| ())
    });
    let mut connector = srv.new_connector();

    let request = client::ClientRequest::get(srv.url("/?qp=5"))
        .finish()
        .unwrap();

    let response = srv.block_on(request.send(&mut connector)).unwrap();
    assert!(response.status().is_success());
}
