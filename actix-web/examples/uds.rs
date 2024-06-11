use actix_web::{get, web, HttpRequest};
#[cfg(unix)]
use actix_web::{middleware, App, Error, HttpResponse, HttpServer};

#[get("/resource1/{name}/index.html")]
async fn index(req: HttpRequest, name: web::Path<String>) -> String {
    println!("REQ: {:?}", req);
    format!("Hello: {}!\r\n", name)
}

#[cfg(unix)]
async fn index_async(req: HttpRequest) -> Result<&'static str, Error> {
    println!("REQ: {:?}", req);
    Ok("Hello world!\r\n")
}

#[get("/")]
async fn no_params() -> &'static str {
    "Hello world!\r\n"
}

#[cfg(unix)]
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    HttpServer::new(|| {
        App::new()
            .wrap(middleware::DefaultHeaders::new().add(("X-Version", "0.2")))
            .wrap(middleware::Compress::default())
            .wrap(middleware::Logger::default())
            .service(index)
            .service(no_params)
            .service(
                web::resource("/resource2/index.html")
                    .wrap(middleware::DefaultHeaders::new().add(("X-Version-R2", "0.3")))
                    .default_service(web::route().to(HttpResponse::MethodNotAllowed))
                    .route(web::get().to(index_async)),
            )
            .service(web::resource("/test1.html").to(|| async { "Test\r\n" }))
    })
    .bind_uds("/Users/me/uds-test")?
    .workers(1)
    .run()
    .await
}

#[cfg(not(unix))]
fn main() {}
