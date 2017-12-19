//! Actix web diesel example
//!
//! Diesel does not support tokio, so we have to run it in separate threads.
//! Actix supports sync actors by default, so we going to create sync actor that will
//! use diesel. Technically sync actors are worker style actors, multiple of them
//! can run in parallel and process messages from same queue.
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate diesel;
extern crate uuid;
extern crate futures;
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix_web::*;
use actix::prelude::*;
use diesel::prelude::*;
use futures::future::{Future, ok};

mod models;
mod schema;

/// State with DbExecutor address
struct State {
    db: SyncAddress<DbExecutor>,
}

/// Async request handler
fn index(req: HttpRequest<State>) -> Box<Future<Item=HttpResponse, Error=Error>> {
    let name = &req.match_info()["name"];

    Box::new(
        req.state().db.call_fut(CreateUser{name: name.to_owned()})
            .and_then(|res| {
                match res {
                    Ok(user) => ok(httpcodes::HTTPOk.build().json(user).unwrap()),
                    Err(_) => ok(httpcodes::HTTPInternalServerError.response())
                }
            })
            .map_err(|e| error::ErrorInternalServerError(e).into()))
}

/// This is db executor actor. We are going to run 3 of them in parallele.
struct DbExecutor(SqliteConnection);

/// This is only message that this actor can handle, but it is easy to extend number of
/// messages.
struct CreateUser {
    name: String,
}

impl ResponseType for CreateUser {
    type Item = models::User;
    type Error = Error;
}

impl Actor for DbExecutor {
    type Context = SyncContext<Self>;
}

impl Handler<CreateUser> for DbExecutor {
    fn handle(&mut self, msg: CreateUser, _: &mut Self::Context)
              -> Response<Self, CreateUser>
    {
        use self::schema::users::dsl::*;

        let uuid = format!("{}", uuid::Uuid::new_v4());
        let new_user = models::NewUser {
            id: &uuid,
            name: &msg.name,
        };

        diesel::insert_into(users)
            .values(&new_user)
            .execute(&self.0)
            .expect("Error inserting person");

        let mut items = users
            .filter(id.eq(&uuid))
            .load::<models::User>(&self.0)
            .expect("Error loading person");

        Self::reply(items.pop().unwrap())
    }
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("diesel-example");

    // Start db executor actors
    let addr = SyncArbiter::start(3, || {
        DbExecutor(SqliteConnection::establish("test.db").unwrap())
    });

    // Start http server
    HttpServer::new(move || {
        Application::with_state(State{db: addr.clone()})
            // enable logger
            .middleware(middlewares::Logger::default())
            .resource("/{name}", |r| r.method(Method::GET).a(index))})
        .bind("127.0.0.1:8080").unwrap()
        .start().unwrap();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
