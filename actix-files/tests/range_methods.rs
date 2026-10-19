use actix_files::NamedFile;
use actix_web::{
    body::{to_bytes, BodySize, MessageBody},
    http::{header, Method, StatusCode},
    test::TestRequest,
};

async fn assert_range_ignored(method: Method, range: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.txt");
    std::fs::write(&path, b"0123456789").unwrap();

    let req = TestRequest::default()
        .method(method)
        .insert_header((header::RANGE, range))
        .to_http_request();
    let res = NamedFile::open(path).unwrap().into_response(&req);

    assert_eq!(res.status(), StatusCode::OK);
    assert!(!res.headers().contains_key(header::CONTENT_RANGE));

    // The HTTP layer uses this size for Content-Length and omits the body for HEAD.
    assert_eq!(res.body().size(), BodySize::Sized(10));
    assert_eq!(to_bytes(res.into_body()).await.unwrap(), &b"0123456789"[..]);
}

#[actix_web::test]
async fn head_ignores_valid_range() {
    assert_range_ignored(Method::HEAD, "bytes=2-4").await;
}

#[actix_web::test]
async fn head_ignores_unsatisfiable_range() {
    assert_range_ignored(Method::HEAD, "bytes=100-200").await;
}

#[actix_web::test]
async fn post_ignores_valid_range() {
    assert_range_ignored(Method::POST, "bytes=2-4").await;
}

#[actix_web::test]
async fn post_ignores_unsatisfiable_range() {
    assert_range_ignored(Method::POST, "bytes=100-200").await;
}
