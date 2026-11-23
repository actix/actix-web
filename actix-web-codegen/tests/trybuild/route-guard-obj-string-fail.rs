use actix_web_codegen::{get, route};

#[get("/", guard_obj = "actix_web::guard::Header(\"x-test\", \"1\")")]
async fn index() -> &'static str {
    "ok"
}

#[route("/", method = "GET", guard_obj = r#"actix_web::guard::Header("x-test", "1")"#)]
async fn raw_string() -> &'static str {
    "ok"
}

fn main() {}
