use actix_files::Files;
use actix_web::{
    http::StatusCode,
    test::{self, TestRequest},
    App,
};

#[actix_rt::test]
async fn test_directory_traversal_prevention() {
    let srv = test::init_service(App::new().service(Files::new("/", "./tests"))).await;

    let req =
        TestRequest::with_uri("/../../../../../../../../../../../etc/passwd").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let req = TestRequest::with_uri(
        "/%2e%2e/%2e%2e/%2e%2e/%2e%2e/%2e%2e/%2e%2e/%2e%2e/%2e%2e/%2e%2e/%2e%2e/etc/passwd",
    )
    .to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let req = TestRequest::with_uri("/%00/etc/passwd%00").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
