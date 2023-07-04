use actix_web::{web, App, Error, HttpResponse, HttpServer, Result};

async fn index<T: ApiProvider>(api: web::Data<T>) -> Result<HttpResponse, Error> {
    let hi = api.dont_worry();
    println!("{}", hi);

    Ok(HttpResponse::Ok().body(hi))
}

pub trait ApiProvider {
    fn dont_worry(&self) -> String;
}

pub struct Api {}

impl ApiProvider for Api {
    fn dont_worry(&self) -> String {
        "be happy".to_string()
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let api = web::Data::new(Api {});

    HttpServer::new(move || {
        App::new()
            .app_data(api.clone())
            .service(web::resource("/").route(web::get().to(index::<Api>)))
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
