use actix_web_codegen::*;
use actix_web::http::Method;
use std::str::FromStr;

#[route("/", method = "hello")]
async fn index() -> String {
    "Hello World!".to_owned()
}

#[actix_web::main]
async fn main() {
    use actix_web::App;

    let srv = actix_test::start(|| App::new().service(index));

    let request = srv.request(Method::from_str("hello").unwrap(), srv.url("/"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}
