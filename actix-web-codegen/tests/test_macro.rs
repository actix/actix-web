use actix_http::HttpService;
use actix_http_test::TestServer;
use actix_web::{get, http, App, HttpResponse, Responder};

#[get("/test")]
fn test() -> impl Responder {
    HttpResponse::Ok()
}

#[test]
fn test_body() {
    let mut srv = TestServer::new(|| HttpService::new(App::new().service(test)));

    let request = srv.request(http::Method::GET, srv.url("/test"));
    let response = srv.block_on(request.send()).unwrap();
    assert!(response.status().is_success());
}
