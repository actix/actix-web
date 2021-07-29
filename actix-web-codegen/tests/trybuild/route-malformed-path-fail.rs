use actix_web_codegen::*;

#[get("/one/{", other)]
async fn one() -> String {
    "Hello World!".to_owned()
}
fn main() {}
