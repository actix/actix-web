#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;
extern crate futures;

use actix::*;
use actix_web::{
    http, middleware, multipart, server,
    App, AsyncResponder, HttpRequest, HttpResponse, HttpMessage, Error};

use futures::{Future, Stream};
use futures::future::{result, Either};


fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>>
{
    println!("{:?}", req);

    req.multipart()            // <- get multipart stream for current request
        .from_err()            // <- convert multipart errors
        .and_then(|item| {     // <- iterate over multipart items
            match item {
                // Handle multipart Field
                multipart::MultipartItem::Field(field) => {
                    println!("==== FIELD ==== {:?}", field);

                    // Field in turn is stream of *Bytes* object
                    Either::A(
                        field.map_err(Error::from)
                            .map(|chunk| {
                                println!("-- CHUNK: \n{}",
                                         std::str::from_utf8(&chunk).unwrap());})
                            .finish())
                },
                multipart::MultipartItem::Nested(mp) => {
                    // Or item could be nested Multipart stream
                    Either::B(result(Ok(())))
                }
            }
        })
        .finish()  // <- Stream::finish() combinator from actix
        .map(|_| HttpResponse::Ok().into())
        .responder()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("multipart-example");

    server::new(
        || App::new()
            .middleware(middleware::Logger::default()) // <- logger
            .resource("/multipart", |r| r.method(http::Method::POST).a(index)))
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Starting http server: 127.0.0.1:8080");
    let _ = sys.run();
}
