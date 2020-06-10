use actix_service::ServiceFactory;
use bytes::Bytes;
use futures_util::future::{self, ok};

use actix_http::{http, HttpService, Request, Response};
use actix_http_test::test_server;

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

#[actix_rt::test]
async fn test_h1_v2() {
    let srv = test_server(move || {
        HttpService::build()
            .finish(|_| future::ok::<_, ()>(Response::Ok().body(STR)))
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.get("/").header("x-test", "111").send();
    let mut response = request.await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));

    let mut response = srv.post("/").send().await.unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = response.body().await.unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[actix_rt::test]
async fn test_connection_close() {
    let srv = test_server(move || {
        HttpService::build()
            .finish(|_| ok::<_, ()>(Response::Ok().body(STR)))
            .tcp()
            .map(|_| ())
    })
    .await;

    let response = srv.get("/").force_close().send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn test_with_query_parameter() {
    let srv = test_server(move || {
        HttpService::build()
            .finish(|req: Request| {
                if req.uri().query().unwrap().contains("qp=") {
                    ok::<_, ()>(Response::Ok().finish())
                } else {
                    ok::<_, ()>(Response::BadRequest().finish())
                }
            })
            .tcp()
            .map(|_| ())
    })
    .await;

    let request = srv.request(http::Method::GET, srv.url("/?qp=5"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}
