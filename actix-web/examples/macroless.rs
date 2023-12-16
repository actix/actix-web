use actix_web::{middleware, rt, web, App, HttpRequest, HttpServer};

async fn index(req: HttpRequest) -> &'static str {
    println!("REQ: {:?}", req);
    "Hello world!\r\n"
}

fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    rt::System::new().block_on(
        HttpServer::new(|| {
            App::new()
                .wrap(middleware::Logger::default())
                .service(web::resource("/").route(web::get().to(index)))
        })
        .bind(("127.0.0.1", 8080))?
        .workers(1)
        .run(),
    )
}
