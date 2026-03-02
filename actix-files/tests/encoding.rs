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

    // range request with accept-encoding still returns no content-encoding header
    let req = TestRequest::with_uri("/")
        .append_header((header::RANGE, "bytes=10-20"))
        .append_header((header::ACCEPT_ENCODING, "gzip"))
        .to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert!(!res.headers().contains_key(header::CONTENT_ENCODING));
}

#[actix_web::test]
async fn test_multiple_directories() {
    // Create test directories
    std::fs::create_dir_all("./tests/test1").unwrap();
    std::fs::create_dir_all("./tests/test2").unwrap();

    // Create test files
    std::fs::write("./tests/test1/test.txt", "File from test1").unwrap();
    std::fs::write("./tests/test2/fallback.txt", "File from test2").unwrap();

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
    std::fs::create_dir_all("./tests/test3").unwrap();
    std::fs::create_dir_all("./tests/test4").unwrap();

    // Create test files
    std::fs::write("./tests/test3/test.txt", "File from test3").unwrap();
    std::fs::write("./tests/test4/fallback.txt", "File from test4").unwrap();

    // Test multiple directories with new_multiple
    let srv = test::init_service(App::new().service(Files::new_multiple(
        "/",
        vec!["./tests/test3", "./tests/test4"],
    )))
    .await;

    // Test file from first directory
    let req = TestRequest::with_uri("/test.txt").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = test::read_body(res).await;
    assert_eq!(&body[..], b"File from test3");

    // Test file from second directory
    let req = TestRequest::with_uri("/fallback.txt").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = test::read_body(res).await;
    assert_eq!(&body[..], b"File from test4");

    // Test non-existent file
    let req = TestRequest::with_uri("/non-existent.txt").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Clean up
    let _ = std::fs::remove_dir_all("./tests/test3");
    let _ = std::fs::remove_dir_all("./tests/test4");
}

#[actix_web::test]
async fn test_multiple_directories_with_index_file() {
    // Create test directories
    std::fs::create_dir_all("./tests/test_index1").unwrap();
    std::fs::create_dir_all("./tests/test_index2").unwrap();

    // Create test files - only second directory has index.html
    std::fs::write("./tests/test_index1/other.txt", "Other file").unwrap();
    std::fs::write(
        "./tests/test_index2/index.html",
        "<html>Index from test2</html>",
    )
    .unwrap();

    // Test multiple directories with index_file - index.html only exists in second directory
    let srv = test::init_service(
        App::new().service(
            Files::new_from_array("/", &["./tests/test_index1", "./tests/test_index2"])
                .index_file("index.html"),
        ),
    )
    .await;

    // Request / should find index.html in second directory
    let req = TestRequest::with_uri("/").to_request();
    let res = test::call_service(&srv, req).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = test::read_body(res).await;
    assert_eq!(&body[..], b"<html>Index from test2</html>");

    // Clean up
    let _ = std::fs::remove_dir_all("./tests/test_index1");
    let _ = std::fs::remove_dir_all("./tests/test_index2");
}

#[actix_web::test]
async fn test_multiple_directories_try_compressed() {
    use actix_web::body::MessageBody;

    // Create test directories
    std::fs::create_dir_all("./tests/test_compress1").unwrap();
    std::fs::create_dir_all("./tests/test_compress2").unwrap();

    // Create test files:
    // - First directory has only uncompressed file
    // - Second directory has both uncompressed and compressed files
    std::fs::copy("./tests/utf8.txt", "./tests/test_compress1/utf8.txt").unwrap();
    std::fs::copy("./tests/utf8.txt", "./tests/test_compress2/other.txt").unwrap();
    std::fs::copy("./tests/utf8.txt.gz", "./tests/test_compress2/other.txt.gz").unwrap();

    let other_txt_gz_len = std::fs::metadata("./tests/test_compress2/other.txt.gz")
        .unwrap()
        .len();

    // Test multiple directories with try_compressed
    let srv = test::init_service(
        App::new().service(
            Files::new_from_array("/", &["./tests/test_compress1", "./tests/test_compress2"])
                .try_compressed(),
        ),
    )
    .await;

    // Request /utf8.txt - first directory has it uncompressed, should return uncompressed
    let mut req = TestRequest::with_uri("/utf8.txt").to_request();
    req.headers_mut().insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("gzip"),
    );
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    // First directory has utf8.txt but no .gz version, so no content-encoding
    assert_eq!(res.headers().get(header::CONTENT_ENCODING), None);

    // Request /other.txt - second directory has both, should return compressed
    let mut req = TestRequest::with_uri("/other.txt").to_request();
    req.headers_mut().insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("gzip"),
    );
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_ENCODING),
        Some(&HeaderValue::from_static("gzip")),
    );
    assert_eq!(
        res.into_body().size(),
        actix_web::body::BodySize::Sized(other_txt_gz_len),
    );

    // Clean up
    let _ = std::fs::remove_dir_all("./tests/test_compress1");
    let _ = std::fs::remove_dir_all("./tests/test_compress2");
}
