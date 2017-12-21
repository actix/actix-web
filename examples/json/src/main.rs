extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate env_logger;
extern crate serde_json;
#[macro_use] extern crate serde_derive;

use actix_web::*;
use futures::Future;

#[derive(Debug, Serialize, Deserialize)]
struct MyObj {
    name: String,
    number: i32,
}

fn index(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    req.json().from_err()
        .and_then(|val: MyObj| {
            println!("model: {:?}", val);
            Ok(httpcodes::HTTPOk.build().json(val)?)  // <- send response
        })
        .responder()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("json-example");

    HttpServer::new(|| {
        Application::new()
            // enable logger
            .middleware(middlewares::Logger::default())
            .resource("/", |r| r.method(Method::POST).f(index))})
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
