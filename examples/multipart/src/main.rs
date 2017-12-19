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

    // get multipart stream and iterate over multipart items
    Box::new(
        req.multipart()
            .map_err(Error::from)
            .and_then(|item| {
                // Multipart stream is a stream of Fields and nested Multiparts
                match item {
                    multipart::MultipartItem::Field(field) => {
                        println!("==== FIELD ==== {:?}", field);

                        // Read field's stream
                        Either::A(
                            field.map_err(Error::from)
                                .map(|chunk| {
                                    println!("-- CHUNK: \n{}",
                                             std::str::from_utf8(&chunk.0).unwrap());})
                                .fold((), |_, _| result::<_, Error>(Ok(()))))
                    },
                    multipart::MultipartItem::Nested(mp) => {
                        // Do nothing for nested multipart stream
                        Either::B(result(Ok(())))
                    }
                }
            })
            // wait until stream finish
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
            // enable logger
            .middleware(middlewares::Logger::default())
            .resource("/multipart", |r| r.method(Method::POST).a(index)))
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Starting http server: 127.0.0.1:8080");
    let _ = sys.run();
}
