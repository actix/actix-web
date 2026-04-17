use actix_web::{web, App, Responder};

use actix_multipart::form::MultipartForm;

#[derive(MultipartForm)]
#[multipart(duplicate_field = "deny")]
struct Form {}

async fn handler(_form: MultipartForm<Form>) -> impl Responder {
    "Hello World!"
}

#[actix_web::main]
async fn main() {
    App::new().default_service(web::to(handler));
}
