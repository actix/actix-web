use std::str::FromStr;

use actix_web::http::Method;
use actix_web_codegen::route;

#[route("/single", method = "CUSTOM")]
async fn index() -> String {
    "Hello Single!".to_owned()
}

#[route("/multi", method = "GET", method = "CUSTOM")]
async fn custom() -> String {
    "Hello Multi!".to_owned()
}

#[actix_web::main]
async fn main() {
    use actix_web::App;

    let srv = actix_test::start(|| App::new().service(index).service(custom));

    let request = srv.request(Method::GET, srv.url("/"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_client_error());

    let request = srv.request(Method::from_str("CUSTOM").unwrap(), srv.url("/single"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(Method::GET, srv.url("/multi"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(Method::from_str("CUSTOM").unwrap(), srv.url("/multi"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}
