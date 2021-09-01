use actix_web::{get, middleware, web::Query, App, HttpServer};
use serde::Deserialize;

#[get("/optional")]
async fn optional_query_params(maybe_qs: Option<Query<OptionalFilters>>) -> String {
    format!("you asked for the optional query params: {:#?}", maybe_qs)
}

#[get("/mandatory")]
async fn mandatory_query_params(maybe_qs: Query<MandatoryFilters>) -> String {
    format!("you asked for the mandatory query params: {:#?}", maybe_qs)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    HttpServer::new(|| {
        App::new()
            .wrap(middleware::Logger::default())
            .service(optional_query_params)
            .service(mandatory_query_params)
    })
    .bind("127.0.0.1:8080")?
    .workers(1)
    .run()
    .await
}

#[derive(Debug, Deserialize)]
pub struct OptionalFilters {
    limit: Option<i32>,
    active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct MandatoryFilters {
    limit: i32,
    active: bool,
}
