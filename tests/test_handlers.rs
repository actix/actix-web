extern crate actix;
extern crate actix_web;
extern crate bytes;
extern crate futures;
extern crate h2;
extern crate http;
extern crate tokio_core;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

use std::time::Duration;

use actix::*;
use actix_web::*;
use bytes::Bytes;
use futures::Future;
use http::StatusCode;
use serde_json::Value;
use tokio_core::reactor::Timeout;

#[derive(Deserialize)]
struct PParam {
    username: String,
}

#[test]
fn test_path_extractor() {
    let mut srv = test::TestServer::new(|app| {
        app.resource("/{username}/index.html", |r| {
            r.with(|p: Path<PParam>| format!("Welcome {}!", p.username))
        });
    });

    // client request
    let request = srv.get()
        .uri(srv.url("/test/index.html"))
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(b"Welcome test!"));
}

#[test]
fn test_query_extractor() {
    let mut srv = test::TestServer::new(|app| {
        app.resource("/index.html", |r| {
            r.with(|p: Query<PParam>| format!("Welcome {}!", p.username))
        });
    });

    // client request
    let request = srv.get()
        .uri(srv.url("/index.html?username=test"))
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(b"Welcome test!"));

    // client request
    let request = srv.get()
        .uri(srv.url("/index.html"))
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_path_and_query_extractor() {
    let mut srv = test::TestServer::new(|app| {
        app.resource("/{username}/index.html", |r| {
            r.route().with2(|p: Path<PParam>, q: Query<PParam>| {
                format!("Welcome {} - {}!", p.username, q.username)
            })
        });
    });

    // client request
    let request = srv.get()
        .uri(srv.url("/test1/index.html?username=test2"))
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(b"Welcome test1 - test2!"));

    // client request
    let request = srv.get()
        .uri(srv.url("/test1/index.html"))
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_path_and_query_extractor2() {
    let mut srv = test::TestServer::new(|app| {
        app.resource("/{username}/index.html", |r| {
            r.route()
                .with3(|_: HttpRequest, p: Path<PParam>, q: Query<PParam>| {
                    format!("Welcome {} - {}!", p.username, q.username)
                })
        });
    });

    // client request
    let request = srv.get()
        .uri(srv.url("/test1/index.html?username=test2"))
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(b"Welcome test1 - test2!"));

    // client request
    let request = srv.get()
        .uri(srv.url("/test1/index.html"))
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_path_and_query_extractor2_async() {
    let mut srv = test::TestServer::new(|app| {
        app.resource("/{username}/index.html", |r| {
            r.route().with3(
                |p: Path<PParam>, _: Query<PParam>, data: Json<Value>| {
                    Timeout::new(Duration::from_millis(10), &Arbiter::handle())
                        .unwrap()
                        .and_then(move |_| {
                            Ok(format!("Welcome {} - {}!", p.username, data.0))
                        })
                        .responder()
                },
            )
        });
    });

    // client request
    let request = srv.post()
        .uri(srv.url("/test1/index.html?username=test2"))
        .header("content-type", "application/json")
        .body("{\"test\": 1}")
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(
        bytes,
        Bytes::from_static(b"Welcome test1 - {\"test\":1}!")
    );
}

#[test]
fn test_path_and_query_extractor3_async() {
    let mut srv = test::TestServer::new(|app| {
        app.resource("/{username}/index.html", |r| {
            r.route().with2(|p: Path<PParam>, data: Json<Value>| {
                Timeout::new(Duration::from_millis(10), &Arbiter::handle())
                    .unwrap()
                    .and_then(move |_| {
                        Ok(format!("Welcome {} - {}!", p.username, data.0))
                    })
                    .responder()
            })
        });
    });

    // client request
    let request = srv.post()
        .uri(srv.url("/test1/index.html"))
        .header("content-type", "application/json")
        .body("{\"test\": 1}")
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_path_and_query_extractor4_async() {
    let mut srv = test::TestServer::new(|app| {
        app.resource("/{username}/index.html", |r| {
            r.route().with2(|data: Json<Value>, p: Path<PParam>| {
                Timeout::new(Duration::from_millis(10), &Arbiter::handle())
                    .unwrap()
                    .and_then(move |_| {
                        Ok(format!("Welcome {} - {}!", p.username, data.0))
                    })
                    .responder()
            })
        });
    });

    // client request
    let request = srv.post()
        .uri(srv.url("/test1/index.html"))
        .header("content-type", "application/json")
        .body("{\"test\": 1}")
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_path_and_query_extractor2_async2() {
    let mut srv = test::TestServer::new(|app| {
        app.resource("/{username}/index.html", |r| {
            r.route().with3(
                |p: Path<PParam>, data: Json<Value>, _: Query<PParam>| {
                    Timeout::new(Duration::from_millis(10), &Arbiter::handle())
                        .unwrap()
                        .and_then(move |_| {
                            Ok(format!("Welcome {} - {}!", p.username, data.0))
                        })
                        .responder()
                },
            )
        });
    });

    // client request
    let request = srv.post()
        .uri(srv.url("/test1/index.html?username=test2"))
        .header("content-type", "application/json")
        .body("{\"test\": 1}")
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(
        bytes,
        Bytes::from_static(b"Welcome test1 - {\"test\":1}!")
    );

    // client request
    let request = srv.get()
        .uri(srv.url("/test1/index.html"))
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_non_ascii_route() {
    let mut srv = test::TestServer::new(|app| {
        app.resource("/中文/index.html", |r| r.f(|_| "success"));
    });

    // client request
    let request = srv.get()
        .uri(srv.url("/中文/index.html"))
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(b"success"));
}

#[test]
fn test_unsafe_path_route() {
    let mut srv = test::TestServer::new(|app| {
        app.resource("/test/{url}", |r| {
            r.f(|r| format!("success: {}", &r.match_info()["url"]))
        });
    });

    // client request
    let request = srv.get()
        .uri(srv.url("/test/http%3A%2F%2Fexample.com"))
        .finish()
        .unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.execute(response.body()).unwrap();
    assert_eq!(
        bytes,
        Bytes::from_static(b"success: http:%2F%2Fexample.com")
    );
}
