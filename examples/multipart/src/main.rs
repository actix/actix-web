#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix::*;
use actix_web::*;

struct MyRoute;

impl Actor for MyRoute {
    type Context = HttpContext<Self>;
}

impl Route for MyRoute {
    type State = ();

    fn request(req: HttpRequest, payload: Payload, ctx: &mut HttpContext<Self>) -> Reply<Self> {
        println!("{:?}", req);

        let multipart = match req.multipart(payload) {
            Ok(mp) => mp,
            Err(e) => return e.into(),
        };

        // get Multipart stream
        WrapStream::<MyRoute>::actstream(multipart)
            .and_then(|item, act, ctx| {
                // Multipart stream is a stream of Fields and nested Multiparts
                match item {
                    multipart::MultipartItem::Field(field) => {
                        println!("==== FIELD ==== {:?}", field);

                        // Read field's stream
                        fut::Either::A(
                            field.actstream()
                                .map(|chunk, act, ctx| {
                                    println!(
                                        "-- CHUNK: \n{}",
                                        std::str::from_utf8(&chunk.0).unwrap());
                                })
                                .finish())
                    },
                    multipart::MultipartItem::Nested(mp) => {
                        // Do nothing for nested multipart stream
                        fut::Either::B(fut::ok(()))
                    }
                }
            })
            // wait until stream finish
            .finish()
            .map_err(|e, act, ctx| {
                ctx.start(httpcodes::HTTPBadRequest);
                ctx.write_eof();
            })
            .map(|_, act, ctx| {
                ctx.start(httpcodes::HTTPOk);
                ctx.write_eof();
            })
            .spawn(ctx);

        Reply::async(MyRoute)
    }
}

fn main() {
    let _ = env_logger::init();
    let sys = actix::System::new("multipart-example");

    HttpServer::new(
        vec![
            Application::default("/")
                .resource("/multipart", |r| {
                    r.post::<MyRoute>();
                }).finish()
        ])
        .serve::<_, ()>("127.0.0.1:8080").unwrap();

    let _ = sys.run();
}
