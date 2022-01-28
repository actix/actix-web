use actix_files::Files;
use actix_web::{
    http::{
        header::{self, HeaderValue},
        StatusCode,
    },
    test::{self, TestRequest},
    App,
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
        test::init_service(App::new().service(Files::new("/", "./tests").prefer_utf8(false)))
            .await;

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

    let srv =
        test::init_service(App::new().service(Files::new("/", "./tests").try_compressed()))
            .await;

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
    assert_eq!(res.into_body().size(), actix_web::body::BodySize::Sized(76),);

    // Select the highest priority encoding
    let mut req = TestRequest::with_uri("/utf8.txt").to_request();
    req.headers_mut().insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("gz;q=0.6,br;q=0.8,*"),
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
    assert_eq!(res.into_body().size(), actix_web::body::BodySize::Sized(49),);

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

    // Can still request a compressed file directly
    let req = TestRequest::with_uri("/utf8.txt.gz").to_request();
    let res = test::call_service(&srv, req).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/x-gzip")),
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
    assert_eq!(res.headers().get(header::CONTENT_ENCODING), None,);
}
