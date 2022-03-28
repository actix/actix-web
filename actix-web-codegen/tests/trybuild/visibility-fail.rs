use actix_web::App;

mod config {
    use actix_web_codegen::*;
    use actix_web::{Responder, HttpResponse};
    
    #[get("/config")]
    async fn config() -> impl Responder {
        HttpResponse::Ok()
    }
}

#[actix_web::main]
async fn main() {
    let srv = actix_test::start(|| App::new().service(config::config));

    let request = srv.get("/config");
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}
