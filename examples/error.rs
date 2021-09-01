use actix_web::{App, HttpServer, get, post, middleware, web::{Json, Query}};
use serde::Deserialize;

#[get("/optional")]
async fn optional_query_params(maybe_qs: Option<Query<OptionalFilters>>) -> String {
    format!("you asked for the optional query params: {:#?}", maybe_qs)
}

#[get("/mandatory")]
async fn mandatory_query_params(maybe_qs: Query<MandatoryFilters>) -> String {
    format!("you asked for the mandatory query params: {:#?}", maybe_qs)
}

#[post("/optional")]
async fn optional_payload(maybe_qs: Option<Query<OptionalFilters>>, maybe_payload: Option<Json<OptionalPayload>>) -> String {
    format!("you asked for the optional query params: {:#?} and optional body: {:#?}", maybe_qs, maybe_payload)
}

#[post("/mandatory")]
async fn mandatory_payload(maybe_qs: Query<MandatoryFilters>, maybe_payload: Json<OptionalPayload>) -> String {
    format!("you asked for the mandatory query params: {:#?} and mandatory body: {:#?}", maybe_qs, maybe_payload)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    HttpServer::new(|| {
        App::new()
            .wrap(middleware::Logger::default())
            .service(optional_query_params)
            .service(mandatory_query_params)
            .service(optional_payload)
            .service(mandatory_payload)
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

#[derive(Debug, Deserialize)]
pub struct OptionalPayload {
    name: Option<String>,
    age: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct MandatoryPayload {
    name: String,
    age: i32,
}
