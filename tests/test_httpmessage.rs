extern crate actix_web;
extern crate http;

use actix_web::*;
use http::{header, Method, Uri, Version, HeaderMap, HttpTryFrom};


#[test]
fn test_no_request_cookies() {
    let mut req = HttpRequest::new(
        Method::GET, Uri::try_from("/").unwrap(), Version::HTTP_11, HeaderMap::new());
    assert!(req.cookies().is_empty());
    let _ = req.load_cookies();
    assert!(req.cookies().is_empty());
}

#[test]
fn test_request_cookies() {
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE,
                   header::HeaderValue::from_static("cookie1=value1; cookie2=value2"));

    let mut req = HttpRequest::new(
        Method::GET, Uri::try_from("/").unwrap(), Version::HTTP_11, headers);
    assert!(req.cookies().is_empty());
    let cookies = req.load_cookies().unwrap();
    assert_eq!(cookies.len(), 2);
    assert_eq!(cookies[0].name(), "cookie1");
    assert_eq!(cookies[0].value(), "value1");
    assert_eq!(cookies[1].name(), "cookie2");
    assert_eq!(cookies[1].value(), "value2");
}
