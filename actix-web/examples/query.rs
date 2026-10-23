use std::io;

use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct SearchQuery {
    user_id: u64,
    status: String,
}

#[derive(Debug, Serialize)]
struct SearchResult {
    found: bool,
}

fn search(query: SearchQuery) -> SearchResult {
    SearchResult {
        found: query.user_id == 42 && query.status == "active",
    }
}

async fn search_get(query: web::Query<SearchQuery>) -> impl Responder {
    let result = search(query.into_inner());

    HttpResponse::Ok()
        .insert_header(("Accept-Query", mime::APPLICATION_JSON))
        .json(result)
}

async fn search_query(web::Json(form): web::Json<SearchQuery>) -> impl Responder {
    let result = search(form);

    HttpResponse::Ok().json(result)
}

#[tokio::main]
async fn main() -> io::Result<()> {
    HttpServer::new(|| {
        App::new().service(web::resource("/search").get(search_get).query(search_query))
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
