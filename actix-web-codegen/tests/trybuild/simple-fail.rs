use actix_web::*;

#[get("/one", other)]
async fn one() -> impl Responder {
    HttpResponse::Ok()
}

#[post(/two)]
async fn two() -> impl Responder {
    HttpResponse::Ok()
}

static PATCH_PATH: &str = "/three";

#[patch(PATCH_PATH)]
async fn three() -> impl Responder {
    HttpResponse::Ok()
}

#[delete("/four", "/five")]
async fn four() -> impl Responder {
    HttpResponse::Ok()
}

#[delete("/five", method="GET")]
async fn five() -> impl Responder {
    HttpResponse::Ok()
}

fn main() {}
