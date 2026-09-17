//! Reproduction of https://github.com/actix/actix-web/issues/3202.

use std::time::Duration;

use actix_multipart::{
    form::{tempfile::TempFile, FieldReader, Limits},
    Multipart,
};
use actix_web::{
    dev, error::ErrorBadRequest, test::TestRequest, web, App, FromRequest, HttpResponse,
};
use futures_util::StreamExt as _;
use tokio::time::timeout;

async fn upload(req: dev::ServiceRequest) -> actix_web::Result<dev::ServiceResponse> {
    let (req, mut payload) = req.into_parts();

    let mut multipart = Multipart::from_request(&req, &mut payload).await?;
    let field = multipart
        .next()
        .await
        .ok_or_else(|| ErrorBadRequest("no file"))??;

    let mut limits = Limits::new(1000, 1000);
    let file = TempFile::read_field(&req, field, &mut limits).await?;

    Ok(dev::ServiceResponse::new(
        req,
        HttpResponse::Ok().body(file.size.to_string()),
    ))
}

fn body(tail: &str) -> String {
    format!("--xxx\r\nContent-Disposition: form-data; name=\"my_uploaded_file\"; filename=\"test.txt\"\r\n\r\n{tail}")
}

async fn check_local(tail: &str, valid: bool) {
    let req = TestRequest::post()
        .insert_header(("content-type", "multipart/form-data; boundary=xxx"))
        .set_payload(body(tail))
        .to_srv_request();

    let result = timeout(Duration::from_secs(1), upload(req))
        .await
        .expect("TempFile::read_field did not finish within one second");
    assert_eq!(
        result.is_ok(),
        valid,
        "unexpected upload result: {result:?}"
    );
}

#[actix_web::test]
async fn missing_boundary_returns_error() {
    check_local("Hello World\r\n", false).await;
}

#[actix_web::test]
async fn malformed_empty_file_returns_error() {
    check_local("--xxx--\r\n", false).await;
}

#[actix_web::test]
async fn partial_boundary_returns_error() {
    check_local("\r\n--x", false).await;
}

#[actix_web::test]
async fn valid_empty_file_succeeds() {
    check_local("\r\n--xxx--\r\n", true).await;
}

#[actix_web::test]
async fn live_missing_boundary_returns_bad_request() {
    check_live("Hello World\r\n", false).await;
}

#[actix_web::test]
async fn live_malformed_empty_file_returns_bad_request() {
    check_live("--xxx--\r\n", false).await;
}

#[actix_web::test]
async fn live_valid_empty_file_succeeds() {
    check_live("\r\n--xxx--\r\n", true).await;
}

async fn check_live(tail: &str, valid: bool) {
    let server = actix_test::start(|| App::new().service(web::service("/").finish(upload)));
    let response = server
        .post("/")
        .insert_header(("content-type", "multipart/form-data; boundary=xxx"))
        .timeout(Duration::from_secs(1))
        .send_body(body(tail))
        .await
        .expect("live upload did not return a response");

    assert_eq!(response.status().as_u16(), if valid { 200 } else { 400 });
}

#[actix_web::test]
async fn missing_boundary_without_trailing_crlf_returns_error() {
    check_local("Hello World", false).await;
}

#[actix_web::test]
async fn raw_field_with_trailing_crlf_returns_error() {
    let (req, mut payload) = TestRequest::post()
        .insert_header(("content-type", "multipart/form-data; boundary=xxx"))
        .set_payload(body("\r\n"))
        .to_http_parts();

    let mut multipart = Multipart::from_request(&req, &mut payload).await.unwrap();
    let mut field = multipart.next().await.unwrap().unwrap();

    let result = timeout(Duration::from_secs(1), field.next())
        .await
        .expect("raw Field stream did not finish at EOF");
    assert!(matches!(
        result,
        Some(Err(actix_multipart::MultipartError::Incomplete))
    ));
}
