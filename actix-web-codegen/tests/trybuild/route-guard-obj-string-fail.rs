use actix_web_codegen::get;

#[get("/", guard_obj = "actix_web::guard::Header(\"x-test\", \"1\")")]
async fn index() -> &'static str {
    "ok"
}

fn main() {}
