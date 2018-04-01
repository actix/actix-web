//! Actix web diesel example
//!
//! Diesel does not support tokio, so we have to run it in separate threads.
//! Actix supports sync actors by default, so we going to create sync actor that use diesel.
//! Technically sync actors are worker style actors, multiple of them
//! can run in parallel and process messages from same queue.
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate diesel;
extern crate r2d2;
extern crate r2d2_diesel;
extern crate uuid;
extern crate futures;
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix::prelude::*;
use actix_web::{http, middleware,
                App, HttpServer, HttpRequest, HttpResponse, Error, AsyncResponder};

use diesel::prelude::*;
use r2d2_diesel::ConnectionManager;
use futures::future::Future;

mod db;
mod models;
mod schema;

use db::{CreateUser, DbExecutor};


/// State with DbExecutor address
struct State {
    db: Addr<Syn, DbExecutor>,
}

/// Async request handler
fn index(req: HttpRequest<State>) -> Box<Future<Item=HttpResponse, Error=Error>> {
    let name = &req.match_info()["name"];

    // send async `CreateUser` message to a `DbExecutor`
    req.state().db.send(CreateUser{name: name.to_owned()})
        .from_err()
        .and_then(|res| {
            match res {
                Ok(user) => Ok(HttpResponse::Ok().json(user)),
                Err(_) => Ok(HttpResponse::InternalServerError().into())
            }
        })
        .responder()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();
    let sys = actix::System::new("diesel-example");

    // Start 3 db executor actors
    let manager = ConnectionManager::<SqliteConnection>::new("test.db");
    let pool = r2d2::Pool::builder().build(manager).expect("Failed to create pool.");

    let addr = SyncArbiter::start(3, move || {
        DbExecutor(pool.clone())
    });

    // Start http server
    let _addr = HttpServer::new(move || {
        App::with_state(State{db: addr.clone()})
            // enable logger
            .middleware(middleware::Logger::default())
            .resource("/{name}", |r| r.method(http::Method::GET).a(index))})
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
