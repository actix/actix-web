use actix_web::*;

#[route("/", method="GET", method="GET")]
async fn index() -> impl Responder {
    HttpResponse::Ok()
}

#[actix_web::main]
async fn main() {
    let srv = test::start(|| App::new().service(index));

    let request = srv.get("/");
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}
