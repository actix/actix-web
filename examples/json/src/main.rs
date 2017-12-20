extern crate actix;
extern crate actix_web;
extern crate bytes;
extern crate futures;
extern crate env_logger;
extern crate serde_json;
#[macro_use] extern crate serde_derive;

use actix_web::*;
use bytes::BytesMut;
use futures::Stream;
use futures::future::{Future, ok, err};

#[derive(Debug, Deserialize)]
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
        req.payload_mut()      // <- load request body
            .readany()
            .fold(BytesMut::new(), |mut body, chunk| {
                body.extend(chunk);
                ok::<_, error::PayloadError>(body)
            })
            .map_err(|e| Error::from(e))
            .and_then(|body| { // <- body is loaded, now we can deserialize json
                match serde_json::from_slice::<MyObj>(&body) {
                    Ok(obj) => {
                        println!("model: {:?}", obj);    // <- do something with payload
                        ok(httpcodes::HTTPOk.response()) // <- send response
                    },
                    Err(e) => {
                        err(error::ErrorBadRequest(e).into())
                    }
                }
            })))
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
