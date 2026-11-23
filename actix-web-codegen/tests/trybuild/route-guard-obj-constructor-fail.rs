use actix_web_codegen::get;

#[get("/", guard_obj = actix_web::guard::Get)]
async fn index() -> &'static str {
    "ok"
}

fn main() {}
