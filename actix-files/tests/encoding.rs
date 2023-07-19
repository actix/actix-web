use actix_files::{Files, NamedFile};
use actix_web::{
    http::{
        header::{self, HeaderValue},
        StatusCode,
    },
    test::{self, TestRequest},
    web, App,
};

#[actix_web::test]
async fn test_utf8_file_contents() {
    // use default ISO-8859-1 encoding
    let srv = test::init_service(App::new().service(Files::new("/", "./tests"))).await;

    let req = TestRequest::with_uri("/utf8.txt").to_request();
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/plain; charset=utf-8")),
    );

    // disable UTF-8 attribute
    let srv =
        test::init_service(App::new().service(Files::new("/", "./tests").prefer_utf8(false))).await;

    let req = TestRequest::with_uri("/utf8.txt").to_request();
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/plain")),
    );
}

#[actix_web::test]
async fn partial_range_response_encoding() {
    let srv = test::init_service(App::new().default_service(web::to(|| async {
        NamedFile::open_async("./tests/test.binary").await.unwrap()
    })))
    .await;

    // range request without accept-encoding returns no content-encoding header
    let req = TestRequest::with_uri("/")
        .append_header((header::RANGE, "bytes=10-20"))
        .to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert!(!res.headers().contains_key(header::CONTENT_ENCODING));

    // range request with accept-encoding returns a content-encoding header
    let req = TestRequest::with_uri("/")
        .append_header((header::RANGE, "bytes=10-20"))
        .append_header((header::ACCEPT_ENCODING, "identity"))
        .to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers().get(header::CONTENT_ENCODING).unwrap(),
        "identity"
    );
}
