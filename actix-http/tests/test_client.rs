use actix_http::{
    error, http, http::StatusCode, HttpMessage, HttpService, Request, Response,
};
use actix_http_test::test_server;
use actix_service::ServiceFactoryExt;
use actix_utils::future;
use bytes::Bytes;
use futures_util::StreamExt as _;

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
            .finish(|_| future::ok::<_, ()>(Response::ok().set_body(STR)))
            .tcp()
    })
    .await;

    let response = srv.get("/").send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.get("/").insert_header(("x-test", "111")).send();
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
            .finish(|_| future::ok::<_, ()>(Response::ok().set_body(STR)))
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
                    future::ok::<_, ()>(Response::ok())
                } else {
                    future::ok::<_, ()>(Response::bad_request())
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

#[actix_rt::test]
async fn test_h1_expect() {
    let srv = test_server(move || {
        HttpService::build()
            .expect(|req: Request| async {
                if req.headers().contains_key("AUTH") {
                    Ok(req)
                } else {
                    Err(error::ErrorExpectationFailed("expect failed"))
                }
            })
            .h1(|req: Request| async move {
                let (_, mut body) = req.into_parts();
                let mut buf = Vec::new();
                while let Some(Ok(chunk)) = body.next().await {
                    buf.extend_from_slice(&chunk);
                }
                let str = std::str::from_utf8(&buf).unwrap();
                assert_eq!(str, "expect body");

                Ok::<_, ()>(Response::ok())
            })
            .tcp()
    })
    .await;

    // test expect without payload.
    let request = srv
        .request(http::Method::GET, srv.url("/"))
        .insert_header(("Expect", "100-continue"));

    let response = request.send().await;
    assert!(response.is_err());

    // test expect would fail to continue
    let request = srv
        .request(http::Method::GET, srv.url("/"))
        .insert_header(("Expect", "100-continue"));

    let response = request.send_body("expect body").await.unwrap();
    assert_eq!(response.status(), StatusCode::EXPECTATION_FAILED);

    // test exepct would continue
    let request = srv
        .request(http::Method::GET, srv.url("/"))
        .insert_header(("Expect", "100-continue"))
        .insert_header(("AUTH", "996"));

    let response = request.send_body("expect body").await.unwrap();
    assert!(response.status().is_success());
}
