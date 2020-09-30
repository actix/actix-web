use actix_files as fs;
use actix_web::{http::header::DispositionType, middleware, App, HttpServer};

use mime;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var(
        "RUST_LOG",
        "actix_files=debug,actix_server=info,actix_web=info",
    );
    env_logger::init();

    fn all_inline(_: &mime::Name<'_>) -> DispositionType {
        DispositionType::Inline
    }

    HttpServer::new(|| {
        App::new()
            .wrap(middleware::DefaultHeaders::new().header("X-Version", "0.2"))
            .wrap(middleware::Compress::default())
            .wrap(middleware::Logger::default())
            .service(
                fs::Files::new("/static", "/home/alex/c2/ontorender/work")
                    .show_files_listing()
                    .use_last_modified(true)
                    .mime_override(all_inline),
            )
    })
    .bind("127.0.0.1:8080")?
    .workers(1)
    .run()
    .await
}
