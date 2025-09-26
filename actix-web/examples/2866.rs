use actix_web::{
    dev::Server,
    get,
    web::{self, Data},
    App, HttpServer, Responder,
};
use serde::Serialize;

#[derive(Debug, Serialize, Clone, Copy)]
pub struct User {
    id: u64,
}

pub trait UserRepository {
    fn get_user(&self) -> User;
}

#[derive(Clone)]
struct UserClient;

impl UserRepository for UserClient {
    fn get_user(&self) -> User {
        User { id: 99 }
    }
}

// when uncommenting following the line, the type checking is unaccepted
// because of cannot infer type parameter T
#[get("/")]
async fn index<T: UserRepository>(client: web::Data<T>) -> impl Responder {
    let user = client.into_inner().get_user();
    web::Json(user)
}

#[get("hello/{who}")]
async fn hello(who: web::Path<String>) -> impl Responder {
    format!("<h1>hello {who}</h1>")
}

pub fn create_server<T: UserRepository + Send + Sync + 'static + Clone>(
    search: T,
) -> Result<Server, std::io::Error> {
    let server = HttpServer::new(move || {
        App::new()
            .app_data(Data::new(search.clone()))
            // .route("/", web::get().to(index::<T>))
            .service(index::<T>(core::marker::PhantomData::<T>))
            .service(hello)
    })
    .bind("127.0.0.1:8080")?
    .run();
    Ok(server)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    println!("\x1b[1;2;36mserving on http://localhost:8080\x1b[0m");
    let user_client = UserClient;
    create_server(user_client).unwrap().await
}
