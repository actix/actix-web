use actix_web_codegen::scope;

#[scope("/api")]
async fn index() -> &'static str {
    "Hello World!"
}

fn main() {}
