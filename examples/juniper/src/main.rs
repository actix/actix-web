//! Actix web juniper example
//!
//! A simple example integrating juniper in actix-web
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate juniper;
extern crate futures;
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix::prelude::*;
use actix_web::{middleware, http, server,
                Application, AsyncResponder,
                HttpRequest, HttpResponse, HttpMessage, Error};
use juniper::http::graphiql::graphiql_source;
use juniper::http::GraphQLRequest;

use futures::future::Future;

mod schema;

use schema::Schema;
use schema::create_schema;

struct State {
    executor: Addr<Syn, GraphQLExecutor>,
}

#[derive(Serialize, Deserialize)]
pub struct GraphQLData(GraphQLRequest);

impl Message for GraphQLData {
    type Result = Result<String, Error>;
}

pub struct GraphQLExecutor {
    schema: std::sync::Arc<Schema>
}

impl GraphQLExecutor {
    fn new(schema: std::sync::Arc<Schema>) -> GraphQLExecutor {
        GraphQLExecutor {
            schema: schema,
        }
    }
}

impl Actor for GraphQLExecutor {
    type Context = SyncContext<Self>;
}

impl Handler<GraphQLData> for GraphQLExecutor {
    type Result = Result<String, Error>;

    fn handle(&mut self, msg: GraphQLData, _: &mut Self::Context) -> Self::Result {
        let res = msg.0.execute(&self.schema, &());
        let res_text = serde_json::to_string(&res)?;
        Ok(res_text)
    }
}

fn graphiql(_req: HttpRequest<State>) -> Result<HttpResponse, Error>  {
    let html = graphiql_source("http://127.0.0.1:8080/graphql");
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}

fn graphql(req: HttpRequest<State>) -> Box<Future<Item=HttpResponse, Error=Error>> {
    let executor = req.state().executor.clone();
    req.json()
        .from_err()
        .and_then(move |val: GraphQLData| {
            executor.send(val)
                .from_err()
                .and_then(|res| {
                    match res {
                        Ok(user) => Ok(HttpResponse::Ok().body(user)),
                        Err(_) => Ok(HttpResponse::InternalServerError().into())
                    }
                })
        })
        .responder()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("juniper-example");

    let schema = std::sync::Arc::new(create_schema());
    let addr = SyncArbiter::start(3, move || {
        GraphQLExecutor::new(schema.clone())
    });

    // Start http server
    let _ = server::new(move || {
        Application::with_state(State{executor: addr.clone()})
            // enable logger
            .middleware(middleware::Logger::default())
            .resource("/graphql", |r| r.method(http::Method::POST).h(graphql))
            .resource("/graphiql", |r| r.method(http::Method::GET).h(graphiql))})
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
