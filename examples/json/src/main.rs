extern crate actix;
extern crate actix_web;
extern crate bytes;
extern crate futures;
extern crate env_logger;
extern crate serde_json;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate json;

use actix_web::{
    middleware, http, error, server,
    App, AsyncResponder, HttpRequest, HttpResponse, HttpMessage, Error, Json};

use bytes::BytesMut;
use futures::{Future, Stream};
use json::JsonValue;

#[derive(Debug, Serialize, Deserialize)]
struct MyObj {
    name: String,
    number: i32,
}

/// This handler uses `HttpRequest::json()` for loading json object.
fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    req.json()
        .from_err()  // convert all errors into `Error`
        .and_then(|val: MyObj| {
            println!("model: {:?}", val);
            Ok(HttpResponse::Ok().json(val))  // <- send response
        })
        .responder()
}

/// This handler uses json extractor
fn extract_item(item: Json<MyObj>) -> HttpResponse {
    println!("model: {:?}", &item);
    HttpResponse::Ok().json(item.0)  // <- send response
}

const MAX_SIZE: usize = 262_144;  // max payload size is 256k

/// This handler manually load request payload and parse json object
fn index_manual(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    // HttpRequest is stream of Bytes objects
    req
        // `Future::from_err` acts like `?` in that it coerces the error type from
        // the future into the final error type
        .from_err()

        // `fold` will asynchronously read each chunk of the request body and
        // call supplied closure, then it resolves to result of closure
        .fold(BytesMut::new(), move |mut body, chunk| {
            // limit max size of in-memory payload
            if (body.len() + chunk.len()) > MAX_SIZE {
                Err(error::ErrorBadRequest("overflow"))
            } else {
                body.extend_from_slice(&chunk);
                Ok(body)
            }
        })
        // `Future::and_then` can be used to merge an asynchronous workflow with a
        // synchronous workflow
        .and_then(|body| {
            // body is loaded, now we can deserialize serde-json
            let obj = serde_json::from_slice::<MyObj>(&body)?;
            Ok(HttpResponse::Ok().json(obj)) // <- send response
        })
        .responder()
}

/// This handler manually load request payload and parse json-rust
fn index_mjsonrust(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    req.concat2()
        .from_err()
        .and_then(|body| {
            // body is loaded, now we can deserialize json-rust
            let result = json::parse(std::str::from_utf8(&body).unwrap()); // return Result
            let injson: JsonValue = match result { Ok(v) => v, Err(e) => object!{"err" => e.to_string() } };
            Ok(HttpResponse::Ok()
                .content_type("application/json")
                .body(injson.dump()))
        })
        .responder()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();
    let sys = actix::System::new("json-example");

    server::new(|| {
        App::new()
            // enable logger
            .middleware(middleware::Logger::default())
            .resource("/extractor", |r| {
                r.method(http::Method::POST)
                    .with(extract_item)
                    .limit(4096); // <- limit size of the payload
            })
            .resource("/manual", |r| r.method(http::Method::POST).f(index_manual))
            .resource("/mjsonrust", |r| r.method(http::Method::POST).f(index_mjsonrust))
            .resource("/", |r| r.method(http::Method::POST).f(index))})
        .bind("127.0.0.1:8080").unwrap()
        .shutdown_timeout(1)
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
