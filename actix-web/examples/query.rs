use actix_web::{query, web, App, HttpResponse, HttpServer, Responder};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct SearchQuery {
    user_id: u64,
    status: String,
}

#[derive(Serialize)]
struct SearchResult {
    found: bool,
}

#[query("/search")]
async fn search_handler(query: web::Json<SearchQuery>) -> impl Responder {
    HttpResponse::Ok().json(SearchResult {
        found: query.user_id == 42 && query.status == "active",
    })
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().service(search_handler))
        .bind(("127.0.0.1", 8080))?
        .run()
        .await
}
