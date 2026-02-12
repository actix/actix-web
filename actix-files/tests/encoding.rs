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
async fn test_compression_encodings() {
    use actix_web::body::MessageBody;

    let utf8_txt_len = std::fs::metadata("./tests/utf8.txt").unwrap().len();
    let utf8_txt_br_len = std::fs::metadata("./tests/utf8.txt.br").unwrap().len();
    let utf8_txt_gz_len = std::fs::metadata("./tests/utf8.txt.gz").unwrap().len();

    let srv =
        test::init_service(App::new().service(Files::new("/", "./tests").try_compressed())).await;

    // Select the requested encoding when present
    let mut req = TestRequest::with_uri("/utf8.txt").to_request();
    req.headers_mut().insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("gzip"),
    );
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/plain; charset=utf-8")),
    );
    assert_eq!(
        res.headers().get(header::CONTENT_ENCODING),
        Some(&HeaderValue::from_static("gzip")),
    );
    assert_eq!(
        res.headers().get(header::VARY),
        Some(&HeaderValue::from_static("accept-encoding")),
    );
    assert_eq!(
        res.into_body().size(),
        actix_web::body::BodySize::Sized(utf8_txt_gz_len),
    );

    // Select the highest priority encoding
    let mut req = TestRequest::with_uri("/utf8.txt").to_request();
    req.headers_mut().insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("gzip;q=0.6,br;q=0.8,*"),
    );
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/plain; charset=utf-8")),
    );
    assert_eq!(
        res.headers().get(header::CONTENT_ENCODING),
        Some(&HeaderValue::from_static("br")),
    );
    assert_eq!(
        res.headers().get(header::VARY),
        Some(&HeaderValue::from_static("accept-encoding")),
    );
    assert_eq!(
        res.into_body().size(),
        actix_web::body::BodySize::Sized(utf8_txt_br_len),
    );

    // Request encoding that doesn't exist on disk and fallback to no encoding
    let mut req = TestRequest::with_uri("/utf8.txt").to_request();
    req.headers_mut().insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("zstd"),
    );
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/plain; charset=utf-8")),
    );
    assert_eq!(res.headers().get(header::CONTENT_ENCODING), None,);
    assert_eq!(
        res.into_body().size(),
        actix_web::body::BodySize::Sized(utf8_txt_len),
    );

    // Do not select an encoding explicitly refused via q=0
    let mut req = TestRequest::with_uri("/utf8.txt").to_request();
    req.headers_mut().insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("zstd;q=1, gzip;q=0"),
    );
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/plain; charset=utf-8")),
    );
    assert_eq!(res.headers().get(header::CONTENT_ENCODING), None,);
    assert_eq!(
        res.into_body().size(),
        actix_web::body::BodySize::Sized(utf8_txt_len),
    );

    // Can still request a compressed file directly
    let req = TestRequest::with_uri("/utf8.txt.gz").to_request();
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/gzip")),
    );
    assert_eq!(res.headers().get(header::CONTENT_ENCODING), None,);

    // Don't try compressed files
    let srv = test::init_service(App::new().service(Files::new("/", "./tests"))).await;

    let mut req = TestRequest::with_uri("/utf8.txt").to_request();
    req.headers_mut().insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("gzip"),
    );
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/plain; charset=utf-8")),
    );
    assert_eq!(res.headers().get(header::CONTENT_ENCODING), None);
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

#[actix_web::test]
async fn test_multiple_directories() {
    // Create test directories
    std::fs::create_dir_all("./tests/test1").unwrap();
    std::fs::create_dir_all("./tests/test2").unwrap();

    // Create test files
    std::fs::write("./tests/test1/test.txt", "File from test1").unwrap();
    while !std::path::Path::new("./tests/test1/test.txt").exists() {}
    std::fs::write("./tests/test2/fallback.txt", "File from test2").unwrap();
    while !std::path::Path::new("./tests/test2/fallback.txt").exists() {}

    // Test multiple directories with new_from_array
    let srv = test::init_service(App::new().service(Files::new_from_array(
        "/",
        &["./tests/test1", "./tests/test2"],
    )))
    .await;

    // Test file from first directory
    let req = TestRequest::with_uri("/test.txt").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = test::read_body(res).await;
    assert_eq!(&body[..], b"File from test1");

    // Test file from second directory
    let req = TestRequest::with_uri("/fallback.txt").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = test::read_body(res).await;
    assert_eq!(&body[..], b"File from test2");

    // Test non-existent file
    let req = TestRequest::with_uri("/non-existent.txt").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Clean up
    let _ = std::fs::remove_dir_all("./tests/test1");
    let _ = std::fs::remove_dir_all("./tests/test2");
}

#[actix_web::test]
async fn test_multiple_directories_iterator() {
    // Create test directories
    std::fs::create_dir_all("./tests/test1").unwrap();
    std::fs::create_dir_all("./tests/test2").unwrap();

    // Create test files
    std::fs::write("./tests/test1/test.txt", "File from test1").unwrap();
    while !std::path::Path::new("./tests/test1/test.txt").exists() {}
    std::fs::write("./tests/test2/fallback.txt", "File from test2").unwrap();
    while !std::path::Path::new("./tests/test2/fallback.txt").exists() {}

    // Test multiple directories with new_multiple
    let srv = test::init_service(App::new().service(Files::new_multiple(
        "/",
        vec!["./tests/test1", "./tests/test2"],
    )))
    .await;

    // Test file from first directory
    let req = TestRequest::with_uri("/test.txt").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = test::read_body(res).await;
    assert_eq!(&body[..], b"File from test1");

    // Test file from second directory
    let req = TestRequest::with_uri("/fallback.txt").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = test::read_body(res).await;
    assert_eq!(&body[..], b"File from test2");

    // Test non-existent file
    let req = TestRequest::with_uri("/non-existent.txt").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Clean up
    let _ = std::fs::remove_dir_all("./tests/test1");
    let _ = std::fs::remove_dir_all("./tests/test2");
}
