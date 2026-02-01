use actix_multipart::form::{
    json::Json as MpJson, tempfile::TempFile, MultipartForm, MultipartFormConfig,
};
use actix_web::{middleware::Logger, post, App, HttpServer, Responder};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Metadata {
    name: String,
}

#[derive(Debug, MultipartForm)]
struct UploadForm {
    // Note: the form is also subject to the global limits configured using `MultipartFormConfig`.
    #[multipart(limit = "100MB")]
    file: TempFile,
    json: MpJson<Metadata>,
}

#[post("/videos")]
async fn post_video(MultipartForm(form): MultipartForm<UploadForm>) -> impl Responder {
    format!(
        "Uploaded file {}, with size: {}\ntemporary file ({}) was deleted\n",
        form.json.name,
        form.file.size,
        form.file.file.path().display(),
    )
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    HttpServer::new(move || {
        App::new()
            .service(post_video)
            .wrap(Logger::default())
            // Also increase the global total limit to 100MiB.
            .app_data(MultipartFormConfig::default().total_limit(100 * 1024 * 1024))
    })
    .workers(2)
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
