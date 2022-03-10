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
