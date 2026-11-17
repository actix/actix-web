use actix_web::guard;
use actix_web_codegen::*;

#[route("/", method="GET", method="HEAD")]
async fn index() -> String {
    "Hello World!".to_owned()
}

fn guard_function(_: &guard::GuardContext<'_>) -> bool {
    true
}

#[get(
    "/guarded",
    guard_obj = guard::Header("x-test", "1"),
    guard_obj = guard_function,
    guard_obj = |ctx: &guard::GuardContext<'_>| ctx.head().headers.contains_key("x-test")
)]
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
