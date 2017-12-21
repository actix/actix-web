extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate env_logger;
extern crate serde_json;
#[macro_use] extern crate serde_derive;

use actix_web::*;
use futures::{Future, Stream};

#[derive(Debug, Serialize, Deserialize)]
struct MyObj {
    name: String,
    number: i32,
}

fn index(mut req: HttpRequest) -> Result<Box<Future<Item=HttpResponse, Error=Error>>> {
    // check content-type
    if req.content_type() != "application/json" {
        return Err(error::ErrorBadRequest("wrong content-type").into())
    }

    Ok(Box::new(
        // `concat2` will asynchronously read each chunk of the request body and
        // return a single, concatenated, chunk
        req.payload_mut().readany().concat2()
            // `Future::from_err` acts like `?` in that it coerces the error type from
            // the future into the final error type
            .from_err()
            // `Future::and_then` can be used to merge an asynchronous workflow with a
            // synchronous workflow
            .and_then(|body| { // <- body is loaded, now we can deserialize json
                let obj = serde_json::from_slice::<MyObj>(&body).map_err(error::ErrorBadRequest)?;

                println!("model: {:?}", obj);
                Ok(httpcodes::HTTPOk.build().json(obj)?)  // <- send response
            })
    ))
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
