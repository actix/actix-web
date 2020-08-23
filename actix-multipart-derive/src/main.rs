use actix_multipart_derive::MultipartForm;
use actix_web::{post, App, HttpServer};
use bytes::BytesMut;

#[derive(Debug, Clone, Default, MultipartForm)]
struct Form {
    name: String,

    #[multipart(max_size = 8096)]
    file: BytesMut,
}

#[post("/")]
async fn no_params(form: Form) -> &'static str {
    println!("{:?}", &form);

    "Hello world!\r\n"
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().service(no_params))
        .bind("127.0.0.1:8080")?
        .workers(1)
        .run()
        .await
}
