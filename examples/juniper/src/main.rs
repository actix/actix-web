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
use actix_web::{
    middleware, http, server,
    App, AsyncResponder, HttpRequest, HttpResponse, FutureResponse, Error, State, Json};
use juniper::http::graphiql::graphiql_source;
use juniper::http::GraphQLRequest;
use futures::future::Future;

mod schema;

use schema::Schema;
use schema::create_schema;

struct AppState {
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

fn graphiql(_req: HttpRequest<AppState>) -> Result<HttpResponse, Error>  {
    let html = graphiql_source("http://127.0.0.1:8080/graphql");
    Ok(HttpResponse::Ok()
       .content_type("text/html; charset=utf-8")
       .body(html))
}

fn graphql(st: State<AppState>, data: Json<GraphQLData>) -> FutureResponse<HttpResponse> {
    st.executor.send(data.0)
        .from_err()
        .and_then(|res| {
            match res {
                Ok(user) => Ok(HttpResponse::Ok()
                               .content_type("application/json")
                               .body(user)),
                Err(_) => Ok(HttpResponse::InternalServerError().into())
            }
        })
        .responder()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();
    let sys = actix::System::new("juniper-example");

    let schema = std::sync::Arc::new(create_schema());
    let addr = SyncArbiter::start(3, move || {
        GraphQLExecutor::new(schema.clone())
    });

    // Start http server
    server::new(move || {
        App::with_state(AppState{executor: addr.clone()})
            // enable logger
            .middleware(middleware::Logger::default())
            .resource("/graphql", |r| r.method(http::Method::POST).with2(graphql))
            .resource("/graphiql", |r| r.method(http::Method::GET).h(graphiql))})
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
