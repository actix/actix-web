//! Actix web juniper example
//!
//! A simple example integrating juniper in actix-web
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate juniper;
extern crate futures;
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix::*;
use actix_web::*;
use juniper::http::graphiql::graphiql_source;
use juniper::http::GraphQLRequest;

use futures::future::Future;

mod schema;

use schema::Schema;
use schema::create_schema;

lazy_static! {
    static ref SCHEMA: Schema = create_schema();
}

struct State {
    executor: Addr<Syn, GraphQLExecutor>,
}

#[derive(Serialize, Deserialize)]
pub struct GraphQLData(GraphQLRequest);

impl Message for GraphQLData {
    type Result = Result<String, Error>;
}

pub struct GraphQLExecutor;

impl Actor for GraphQLExecutor {
    type Context = SyncContext<Self>;
}

impl Handler<GraphQLData> for GraphQLExecutor {
    type Result = Result<String, Error>;

    fn handle(&mut self, msg: GraphQLData, _: &mut Self::Context) -> Self::Result {
        let res = msg.0.execute(&SCHEMA, &());
        let res_text = serde_json::to_string(&res)?;
        Ok(res_text)
    }
}

fn graphiql(_req: HttpRequest<State>) -> Result<HttpResponse>  {
    let html = graphiql_source("http://localhost:8080/graphql");
    Ok(HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(html).unwrap())
}

fn graphql(req: HttpRequest<State>) -> Box<Future<Item=HttpResponse, Error=Error>> {

    req.json()
        .from_err()
        .and_then(move |val: GraphQLData| {
            req.state().executor.send(val)
                .from_err()
                .and_then(|res| {
                    match res {
                        Ok(user) => Ok(httpcodes::HTTPOk.build().body(user)?),
                        Err(_) => Ok(httpcodes::HTTPInternalServerError.into())
                    }
                })
        })
        .responder()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("juniper-example");

    let addr = SyncArbiter::start(3, || {
        GraphQLExecutor{}
    });

    // Start http server
    let _addr = HttpServer::new(move || {
        Application::with_state(State{executor: addr.clone()})
            // enable logger
            .middleware(middleware::Logger::default())
            .resource("/graphql", |r| r.method(Method::POST).a(graphql))
            .resource("/graphiql", |r| r.method(Method::GET).f(graphiql))})
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
