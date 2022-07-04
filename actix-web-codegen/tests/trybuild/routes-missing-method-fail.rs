use actix_web_codegen::*;

#[routes]
async fn index() -> String {
    "Hello World!".to_owned()
}

#[actix_web::main]
async fn main() {
    use actix_web::App;

    let srv = actix_test::start(|| App::new().service(index));
}
