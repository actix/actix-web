use actix_web_codegen::*;

#[get("/one", other)]
async fn one() -> String {
    "Hello World!".to_owned()
}

#[post(/two)]
async fn two() -> String {
    "Hello World!".to_owned()
}

static PATCH_PATH: &str = "/three";

#[patch(PATCH_PATH)]
async fn three() -> String {
    "Hello World!".to_owned()
}

#[delete("/four", "/five")]
async fn four() -> String {
    "Hello World!".to_owned()
}

#[delete("/five", method="GET")]
async fn five() -> String {
    "Hello World!".to_owned()
}

fn main() {}
