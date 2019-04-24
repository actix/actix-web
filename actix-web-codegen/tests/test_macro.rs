use actix_http::HttpService;
use actix_http_test::TestServer;
use actix_web::{http, App, HttpResponse, Responder};
use actix_web_codegen::get;
use futures::{future, Future};

#[get("/test")]
fn test() -> impl Responder {
    HttpResponse::Ok()
}

#[get("/test")]
fn auto_async() -> impl Future<Item = HttpResponse, Error = actix_web::Error> {
    future::ok(HttpResponse::Ok().finish())
}

#[get("/test")]
fn auto_sync() -> impl Future<Item = HttpResponse, Error = actix_web::Error> {
    future::ok(HttpResponse::Ok().finish())
}

#[test]
fn test_body() {
    let mut srv = TestServer::new(|| HttpService::new(App::new().service(test)));
    let request = srv.request(http::Method::GET, srv.url("/test"));
    let response = srv.block_on(request.send()).unwrap();
    assert!(response.status().is_success());

    let mut srv = TestServer::new(|| HttpService::new(App::new().service(auto_sync)));
    let request = srv.request(http::Method::GET, srv.url("/test"));
    let response = srv.block_on(request.send()).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_auto_async() {
    let mut srv = TestServer::new(|| HttpService::new(App::new().service(auto_async)));

    let request = srv.request(http::Method::GET, srv.url("/test"));
    let response = srv.block_on(request.send()).unwrap();
    assert!(response.status().is_success());
}
