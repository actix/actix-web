use actix_http::HttpService;
use actix_http_test::TestServer;
use actix_web::{get, App, HttpResponse, Responder};

#[get("/test")]
fn test() -> impl Responder {
    HttpResponse::Ok()
}

#[test]
fn test_body() {
    let mut srv = TestServer::new(|| HttpService::new(App::new().service(test)));

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    let response = srv.send_request(request).unwrap();
    assert!(response.status().is_success());
}
