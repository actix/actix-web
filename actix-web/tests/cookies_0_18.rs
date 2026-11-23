#![cfg(feature = "cookies-0_18")]

use actix_web::{cookie_0_18::Cookie, test::TestRequest, HttpResponse, Responder as _};

#[test]
fn request_cookie_encoding() {
    let req = TestRequest::default()
        .cookie_0_18(
            Cookie::build(("session", "hello world"))
                .path("/app")
                .http_only(true)
                .build(),
        )
        .to_srv_request();

    assert_eq!(
        req.headers().get(actix_web::http::header::COOKIE).unwrap(),
        "session=hello%20world"
    );
    assert_eq!(req.cookie_0_18("session").unwrap().value(), "hello world");
    assert_eq!(
        req.cookie_raw_0_18("session").unwrap().value(),
        "hello%20world"
    );
}

#[test]
fn response_cookie_attributes() {
    let cookie = Cookie::build(("session", "value"))
        .partitioned(true)
        .path("/app")
        .build();
    let req = TestRequest::default().to_http_request();
    let mut res = HttpResponse::Ok()
        .finish()
        .customize()
        .add_cookie_0_18(&cookie)
        .respond_to(&req);

    res.add_removal_cookie_0_18(&cookie).unwrap();
    let cookies = res.cookies_0_18().collect::<Vec<_>>();
    assert_eq!(cookies[0].partitioned(), Some(true));
    assert_eq!(cookies[1].path(), Some("/app"));
    assert_eq!(cookies[1].max_age().unwrap().whole_seconds(), 0);
    assert_eq!(res.del_cookie("session"), 2);
}

#[cfg(feature = "cookies")]
#[test]
fn both_cookie_versions_share_headers() {
    use actix_web::cookie::Cookie as LegacyCookie;

    let req = TestRequest::default()
        .cookie(LegacyCookie::new("first", "legacy"))
        .cookie_0_18(Cookie::new("first", "replacement"))
        .cookie(LegacyCookie::new("second", "other"))
        .to_http_request();

    let legacy = req.cookies().unwrap();
    assert_eq!(legacy.len(), 2);
    assert_eq!(req.cookie_0_18("first").unwrap().value(), "replacement");
    assert_eq!(req.cookie_0_18("second").unwrap().value(), "other");

    let res = HttpResponse::Ok()
        .cookie(LegacyCookie::new("legacy", "a"))
        .cookie_0_18(Cookie::new("current", "b"))
        .finish();

    assert_eq!(res.cookies().count(), 2);
    assert_eq!(res.cookies_0_18().count(), 2);
}
