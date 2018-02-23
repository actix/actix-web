//! Actix web juniper example
//!
//! Juniper is a graphql framework implemetation for rust
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate juniper;
extern crate futures;
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix_web::*;
use juniper::http::graphiql::graphiql_source;
use juniper::http::GraphQLRequest;

use futures::future::Future;

mod schema;

use schema::Schema;
use schema::create_schema;

struct State {
    schema: Schema,
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
        .and_then(move |val: GraphQLRequest| {
            let response = val.execute(&req.state().schema, &());
            Ok(httpcodes::HTTPOk.build().json(response)?)
        })
        .responder()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("juniper-example");

    // Start http server
    let _addr = HttpServer::new(move || {
        Application::with_state(State{schema: create_schema() })
            // enable logger
            .middleware(middleware::Logger::default())
            .resource("/graphql", |r| r.method(Method::POST).a(graphql))
            .resource("/graphiql", |r| r.method(Method::GET).f(graphiql))})
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
