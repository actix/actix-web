use actix_web::{web, App, Responder};

use actix_multipart::form::{tempfile::TempFile, MultipartForm};

#[derive(MultipartForm)]
struct Form {
    #[multipart(rename = "files[]")]
    files: Vec<TempFile>,
}

async fn handler(_form: MultipartForm<Form>) -> impl Responder {
    "Hello World!"
}

#[actix_web::main]
async fn main() {
    App::new().default_service(web::to(handler));
}
