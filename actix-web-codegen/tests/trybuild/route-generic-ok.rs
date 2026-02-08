use actix_web::{get, web, App};

trait UserRepository {
    fn get_user(&self) -> u64;
}

#[derive(Clone)]
struct UserClient;

impl UserRepository for UserClient {
    fn get_user(&self) -> u64 {
        99
    }
}

#[derive(Clone)]
struct Flag;

#[get("/")]
async fn index<T>(client: web::Data<T>) -> String
where
    T: UserRepository + Send + Sync + 'static,
{
    client.get_ref().get_user().to_string()
}

#[get("/multi")]
async fn multi<T, U>(client: web::Data<T>, _flag: web::Data<U>) -> String
where
    T: UserRepository + Send + Sync + 'static,
    U: Clone + Send + Sync + 'static,
{
    client.get_ref().get_user().to_string()
}

#[get("/const")]
async fn with_const<const N: usize>() -> String {
    format!("{N}")
}

fn main() {
    let app = App::new()
        .app_data(web::Data::new(UserClient))
        .app_data(web::Data::new(Flag))
        .service(index::<UserClient>::default())
        .service(multi::<UserClient, Flag>::default())
        .service(with_const::<3>::default());

    let _ = app;
}
