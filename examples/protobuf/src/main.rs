extern crate actix;
extern crate actix_web;
extern crate bytes;
extern crate futures;
extern crate env_logger;
extern crate prost;
#[macro_use] 
extern crate prost_derive;

use actix_web::*;
use actix_web::ProtoBufBody;
use futures::Future;


#[derive(Clone, Debug, PartialEq, Message)]
pub struct MyObj {
    #[prost(int32, tag="1")]
    pub number: i32,
    #[prost(string, tag="2")]
    pub name: String,
}


/// This handler uses `HttpRequest::json()` for loading serde json object.
fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    ProtoBufBody::new(req)
        .from_err()  // convert all errors into `Error`
        .and_then(|val: MyObj| {
            println!("model: {:?}", val);
            Ok(httpcodes::HTTPOk.build().protobuf(val)?)  // <- send response
        })
        .responder()
}


fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("protobuf-example");

    let addr = HttpServer::new(|| {
        Application::new()
            .middleware(middleware::Logger::default())
            .resource("/", |r| r.method(Method::POST).f(index))})
        .bind("127.0.0.1:8080").unwrap()
        .shutdown_timeout(1)
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
