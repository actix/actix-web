#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;
extern crate futures;

use actix_web::*;
use futures::{Future, Stream};
use futures::future::{result, Either};


fn index(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>>
{
    println!("{:?}", req);

    Box::new(
        req.multipart()            // <- get multipart stream for current request
            .map_err(Error::from)  // <- convert multipart errors
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
                                .fold((), |_, _| result::<_, Error>(Ok(()))))
                    },
                    multipart::MultipartItem::Nested(mp) => {
                        // Or item could be nested Multipart stream
                        Either::B(result(Ok(())))
                    }
                }
            })
            // wait until stream finishes
            .fold((), |_, _| result::<_, Error>(Ok(())))
            .map(|_| httpcodes::HTTPOk.response())
    )
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("multipart-example");

    HttpServer::new(
        || Application::new()
            .middleware(middlewares::Logger::default()) // <- logger
            .resource("/multipart", |r| r.method(Method::POST).a(index)))
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Starting http server: 127.0.0.1:8080");
    let _ = sys.run();
}
