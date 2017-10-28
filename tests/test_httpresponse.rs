extern crate actix_web;
extern crate http;
extern crate time;

use actix_web::*;
use time::Duration;
use http::{header, Method, Version, HeaderMap};


#[test]
fn test_response_cookies() {
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE,
                   header::HeaderValue::from_static("cookie1=value1; cookie2=value2"));

    let mut req = HttpRequest::new(
        Method::GET, "/".to_owned(), Version::HTTP_11, headers, String::new());
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
