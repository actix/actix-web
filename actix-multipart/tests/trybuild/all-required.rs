use actix_web::{web, App, Responder};

use actix_multipart::form::{tempfile::TempFile, text::Text, MultipartForm};

#[derive(Debug, MultipartForm)]
struct ImageUpload {
    description: Text<String>,
    timestamp: Text<i64>,
    image: TempFile,
}

async fn handler(_form: MultipartForm<ImageUpload>) -> impl Responder {
    "Hello World!"
}

#[actix_web::main]
async fn main() {
    App::new().default_service(web::to(handler));
}
