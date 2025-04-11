use actix_web::{
    get, middleware, post,
    web::{Json, Query},
    App, HttpServer,
};
use serde::Deserialize;

#[get("/optional")]
async fn optional_query_params(maybe_qs: Option<Query<OptionalFilters>>) -> String {
    format!("you asked for the optional query params: {:#?}", maybe_qs)
}

#[get("/mandatory")]
async fn mandatory_query_params(qs: Query<MandatoryFilters>) -> String {
    format!("you asked for the mandatory query params: {:#?}", qs)
}

#[post("/optional")]
async fn optional_payload(
    maybe_qs: Option<Query<OptionalFilters>>,
    maybe_payload: Option<Json<OptionalPayload>>,
) -> String {
    format!(
        "you asked for the optional query params: {:#?} and optional body: {:#?}",
        maybe_qs, maybe_payload
    )
}

#[post("/mandatory")]
async fn mandatory_payload(qs: Query<MandatoryFilters>, payload: Json<MandatoryPayload>) -> String {
    format!(
        "you asked for the mandatory query params: {:#?} and mandatory body: {:#?}",
        qs, payload
    )
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
    pub limit: Option<i32>,
    pub active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct MandatoryFilters {
    pub limit: i32,
    pub active: bool,
}

#[derive(Debug, Deserialize)]
pub struct OptionalPayload {
    pub name: Option<String>,
    pub age: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct MandatoryPayload {
    pub name: String,
    pub age: i32,
}
