use actix_files::Files;
use actix_web::{get, guard, middleware, App, HttpServer, Responder};

const EXAMPLES_DIR: &str = concat![env!("CARGO_MANIFEST_DIR"), "/examples"];

#[get("/")]
async fn index() -> impl Responder {
    "Hello world!"
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    log::info!("starting HTTP server at http://localhost:8080");

    HttpServer::new(|| {
        App::new()
            .service(index)
            .service(
                Files::new("/assets", EXAMPLES_DIR)
                    .show_files_listing()
                    .guard(guard::Header("show-listing", "?1")),
            )
            .service(Files::new("/assets", EXAMPLES_DIR))
            .wrap(middleware::Compress::default())
            .wrap(middleware::Logger::default())
    })
    .bind(("127.0.0.1", 8080))?
    .workers(2)
    .run()
    .await
}
