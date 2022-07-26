use actix_web_codegen::*;

#[routes]
#[get("/")]
#[post("/")]
async fn index() -> String {
    "Hello World!".to_owned()
}

#[actix_web::main]
async fn main() {
    use actix_web::App;

    let srv = actix_test::start(|| App::new().service(index));

    let request = srv.get("/");
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.post("/");
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}
