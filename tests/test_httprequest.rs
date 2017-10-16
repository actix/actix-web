extern crate actix_web;
extern crate http;
extern crate time;

use std::str;
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
    {
        let cookies = req.load_cookies().unwrap();
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0].name(), "cookie1");
        assert_eq!(cookies[0].value(), "value1");
        assert_eq!(cookies[1].name(), "cookie2");
        assert_eq!(cookies[1].value(), "value2");
    }

    let cookie = req.cookie("cookie1");
    assert!(cookie.is_some());
    let cookie = cookie.unwrap();
    assert_eq!(cookie.name(), "cookie1");
    assert_eq!(cookie.value(), "value1");

    let cookie = req.cookie("cookie-unknown");
    assert!(cookie.is_none());
}

#[test]
fn test_no_request_range_header() {
    let req = HttpRequest::new(Method::GET, Uri::try_from("/").unwrap(),
                               Version::HTTP_11, HeaderMap::new());
    let ranges = req.range(100).unwrap();
    assert!(ranges.is_empty());
}

#[test]
fn test_request_range_header() {
    let mut headers = HeaderMap::new();
    headers.insert(header::RANGE,
                   header::HeaderValue::from_static("bytes=0-4"));

    let req = HttpRequest::new(Method::GET, Uri::try_from("/").unwrap(),
                               Version::HTTP_11, headers);
    let ranges = req.range(100).unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].start, 0);
    assert_eq!(ranges[0].length, 5);
}

#[test]
fn test_request_query() {
    let req = HttpRequest::new(Method::GET, Uri::try_from("/?id=test").unwrap(),
                               Version::HTTP_11, HeaderMap::new());

    assert_eq!(req.query_string(), "id=test");
    let query: Vec<_> = req.query().collect();
    assert_eq!(query[0].0.as_ref(), "id");
    assert_eq!(query[0].1.as_ref(), "test");
}

#[test]
fn test_request_match_info() {
    let req = HttpRequest::new(Method::GET, Uri::try_from("/?id=test").unwrap(),
                               Version::HTTP_11, HeaderMap::new());

    let mut params = Params::new();
    params.insert("key".to_owned(), "value".to_owned());

    let req = req.with_match_info(params);
    assert_eq!(req.match_info().find("key"), Some("value"));
}

#[test]
fn test_chunked() {
    let req = HttpRequest::new(
        Method::GET, Uri::try_from("/").unwrap(), Version::HTTP_11, HeaderMap::new());
    assert!(!req.chunked().unwrap());

    let mut headers = HeaderMap::new();
    headers.insert(header::TRANSFER_ENCODING,
                   header::HeaderValue::from_static("chunked"));
    let req = HttpRequest::new(
        Method::GET, Uri::try_from("/").unwrap(), Version::HTTP_11, headers);
    assert!(req.chunked().unwrap());

    let mut headers = HeaderMap::new();
    let s = unsafe{str::from_utf8_unchecked(b"some va\xadscc\xacas0xsdasdlue".as_ref())};

    headers.insert(header::TRANSFER_ENCODING,
                   header::HeaderValue::from_str(s).unwrap());
    let req = HttpRequest::new(
        Method::GET, Uri::try_from("/").unwrap(), Version::HTTP_11, headers);
    assert!(req.chunked().is_err());
}
