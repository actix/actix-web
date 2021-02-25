use actix_web::{Responder, HttpResponse, App, test};
use actix_web_codegen::*;

/// Docstrings shouldn't break anything.
#[get("/")]
async fn index() -> impl Responder {
    HttpResponse::Ok()
}

#[actix_web::main]
async fn main() {
    let srv = test::start(|| App::new().service(index));

    let request = srv.get("/");
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}
