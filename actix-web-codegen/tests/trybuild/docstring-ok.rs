use actix_web::{Responder, HttpResponse, App};
use actix_web_codegen::*;

/// doc comments shouldn't break anything
#[get("/")]
async fn index() -> impl Responder {
    HttpResponse::Ok()
}

#[actix_web::main]
async fn main() {
    let srv = actix_test::start(|| App::new().service(index));

    let request = srv.get("/");
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}
