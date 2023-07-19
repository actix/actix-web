use actix_files::Files;
use actix_web::{
    guard::Host,
    http::StatusCode,
    test::{self, TestRequest},
    App,
};
use bytes::Bytes;

#[actix_web::test]
async fn test_guard_filter() {
    let srv = test::init_service(
        App::new()
            .service(Files::new("/", "./tests/fixtures/guards/first").guard(Host("first.com")))
            .service(Files::new("/", "./tests/fixtures/guards/second").guard(Host("second.com"))),
    )
    .await;

    let req = TestRequest::with_uri("/index.txt")
        .append_header(("Host", "first.com"))
        .to_request();
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(test::read_body(res).await, Bytes::from("first"));

    let req = TestRequest::with_uri("/index.txt")
        .append_header(("Host", "second.com"))
        .to_request();
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(test::read_body(res).await, Bytes::from("second"));
}
