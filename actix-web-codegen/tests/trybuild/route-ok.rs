use actix_web_codegen::*;

#[route("/", method="GET", method="HEAD")]
async fn index() -> String {
    "Hello World!".to_owned()
}

#[get("/guarded", guard_obj = "actix_web::guard::Header(\"x-test\", \"1\")")]
async fn guarded() -> String {
    "Guarded".to_owned()
}

#[actix_web::main]
async fn main() {
    use actix_web::App;

    let srv = actix_test::start(|| App::new().service(index).service(guarded));

    let request = srv.get("/");
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}
