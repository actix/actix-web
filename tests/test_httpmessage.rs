extern crate actix_web;
extern crate http;
extern crate time;

use actix_web::*;
use time::Duration;
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

    let cookie = req.cookie("cookie1");
    assert!(cookie.is_some());
    let cookie = cookie.unwrap();
    assert_eq!(cookie.name(), "cookie1");
    assert_eq!(cookie.value(), "value1");
}

#[test]
fn test_response_cookies() {
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE,
                   header::HeaderValue::from_static("cookie1=value1; cookie2=value2"));

    let mut req = HttpRequest::new(
        Method::GET, Uri::try_from("/").unwrap(), Version::HTTP_11, headers);
    let cookies = req.load_cookies().unwrap();

    let resp = httpcodes::HTTPOk
        .builder()
        .cookie(Cookie::build("name", "value")
                .domain("www.rust-lang.org")
                .path("/test")
                .http_only(true)
                .max_age(Duration::days(1))
                .finish())
        .del_cookie(&cookies[0])
        .body(Body::Empty);

    assert!(resp.is_ok());
    let resp = resp.unwrap();

    let mut val: Vec<_> = resp.headers().get_all("Set-Cookie")
        .iter().map(|v| v.to_str().unwrap().to_owned()).collect();
    val.sort();
    assert!(val[0].starts_with("cookie1=; Max-Age=0;"));
    assert_eq!(
        val[1],"name=value; HttpOnly; Path=/test; Domain=www.rust-lang.org; Max-Age=86400");
}
