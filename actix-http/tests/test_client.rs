use actix_service::NewService;
use bytes::Bytes;
use futures::future::{self, ok};

use actix_http::{http, HttpService, Request, Response};
use actix_http_test::TestServer;

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
    env_logger::init();
    let mut srv = TestServer::new(move || {
        HttpService::build().finish(|_| future::ok::<_, ()>(Response::Ok().body(STR)))
    });
    let response = srv.block_on(srv.get("/").send()).unwrap();
    assert!(response.status().is_success());

    let request = srv.get("/").header("x-test", "111").send();
    let response = srv.block_on(request).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    let response = srv.block_on(srv.post("/").send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.load_body(response).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_connection_close() {
    let mut srv = TestServer::new(move || {
        HttpService::build()
            .finish(|_| ok::<_, ()>(Response::Ok().body(STR)))
            .map(|_| ())
    });
    let response = srv.block_on(srv.get("/").force_close().send()).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_with_query_parameter() {
    let mut srv = TestServer::new(move || {
        HttpService::build()
            .finish(|req: Request| {
                if req.uri().query().unwrap().contains("qp=") {
                    ok::<_, ()>(Response::Ok().finish())
                } else {
                    ok::<_, ()>(Response::BadRequest().finish())
                }
            })
            .map(|_| ())
    });

    let request = srv.request(http::Method::GET, srv.url("/?qp=5")).send();
    let response = srv.block_on(request).unwrap();
    assert!(response.status().is_success());
}
