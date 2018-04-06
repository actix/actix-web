extern crate actix;
extern crate actix_web;
extern crate bytes;
extern crate futures;
#[macro_use]
extern crate failure;
extern crate env_logger;
extern crate prost;
#[macro_use]
extern crate prost_derive;

use futures::Future;
use actix_web::{
    http, middleware, server,
    App, AsyncResponder, HttpRequest, HttpResponse, Error};

mod protobuf;
use protobuf::ProtoBufResponseBuilder;


#[derive(Clone, Debug, PartialEq, Message)]
pub struct MyObj {
    #[prost(int32, tag="1")]
    pub number: i32,
    #[prost(string, tag="2")]
    pub name: String,
}


/// This handler uses `ProtoBufMessage` for loading protobuf object.
fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    protobuf::ProtoBufMessage::new(req)
        .from_err()  // convert all errors into `Error`
        .and_then(|val: MyObj| {
            println!("model: {:?}", val);
            Ok(HttpResponse::Ok().protobuf(val)?)  // <- send response
        })
        .responder()
}


fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();
    let sys = actix::System::new("protobuf-example");

    server::new(|| {
        App::new()
            .middleware(middleware::Logger::default())
            .resource("/", |r| r.method(http::Method::POST).f(index))})
        .bind("127.0.0.1:8080").unwrap()
        .shutdown_timeout(1)
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
