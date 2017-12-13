extern crate actix_web;
extern crate http;
extern crate time;

use std::str;
use std::str::FromStr;
use std::collections::HashMap;
use actix_web::*;
use actix_web::dev::*;
use http::{header, Method, Version, HeaderMap, Uri};


#[test]
fn test_debug() {
    let req = HttpRequest::new(
        Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, HeaderMap::new(), None);
    let _ = format!("{:?}", req);
}

#[test]
fn test_no_request_cookies() {
    let req = HttpRequest::new(
        Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, HeaderMap::new(), None);
    assert!(req.cookies().unwrap().is_empty());
}

#[test]
fn test_request_cookies() {
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE,
                   header::HeaderValue::from_static("cookie1=value1; cookie2=value2"));

    let req = HttpRequest::new(
        Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, headers, None);
    {
        let cookies = req.cookies().unwrap();
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
    let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                               Version::HTTP_11, HeaderMap::new(), None);
    let ranges = req.range(100).unwrap();
    assert!(ranges.is_empty());
}

#[test]
fn test_request_range_header() {
    let mut headers = HeaderMap::new();
    headers.insert(header::RANGE,
                   header::HeaderValue::from_static("bytes=0-4"));

    let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                               Version::HTTP_11, headers, None);
    let ranges = req.range(100).unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].start, 0);
    assert_eq!(ranges[0].length, 5);
}

#[test]
fn test_request_query() {
    let req = HttpRequest::new(Method::GET, Uri::from_str("/?id=test").unwrap(),
                               Version::HTTP_11, HeaderMap::new(), None);
    assert_eq!(req.query_string(), "id=test");
    let query = req.query();
    assert_eq!(&query["id"], "test");
}

#[test]
fn test_request_match_info() {
    let mut req = HttpRequest::new(Method::GET, Uri::from_str("/value/?id=test").unwrap(),
                                   Version::HTTP_11, HeaderMap::new(), None);

    let mut resource = Resource::default();
    resource.name("index");
    let mut map = HashMap::new();
    map.insert(Pattern::new("index", "/{key}/"), Some(resource));
    let router = Router::new("", map);
    assert!(router.recognize(&mut req).is_some());

    assert_eq!(req.match_info().get("key"), Some("value"));
}

#[test]
fn test_chunked() {
    let req = HttpRequest::new(
        Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, HeaderMap::new(), None);
    assert!(!req.chunked().unwrap());

    let mut headers = HeaderMap::new();
    headers.insert(header::TRANSFER_ENCODING,
                   header::HeaderValue::from_static("chunked"));
    let req = HttpRequest::new(
        Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, headers, None);
    assert!(req.chunked().unwrap());

    let mut headers = HeaderMap::new();
    let s = unsafe{str::from_utf8_unchecked(b"some va\xadscc\xacas0xsdasdlue".as_ref())};

    headers.insert(header::TRANSFER_ENCODING,
                   header::HeaderValue::from_str(s).unwrap());
    let req = HttpRequest::new(
        Method::GET, Uri::from_str("/").unwrap(),
        Version::HTTP_11, headers, None);
    assert!(req.chunked().is_err());
}
