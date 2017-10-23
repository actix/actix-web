#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix_web::*;

/// somple handle
fn index(req: &mut HttpRequest, _payload: Payload, state: &()) -> HttpResponse {
    println!("{:?}", req);
    httpcodes::HTTPOk.into()
}

/// handle with path parameters like `/name/{name}/`
fn with_param(req: &mut HttpRequest, _payload: Payload, state: &()) -> HttpResponse {
    println!("{:?}", req);

    HttpResponse::builder(StatusCode::OK)
        .content_type("test/plain")
        .body(Body::Binary(
            format!("Hello {}!", req.match_info().get("name").unwrap()).into())).unwrap()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("ws-example");

    HttpServer::new(
        Application::default("/")
            // enable logger
            .middleware(Logger::new(None))
            // register simple handler, handle all methods
            .handler("/index.html", index)
            // with path parameters
            .resource("/user/{name}/", |r| r.handler(Method::GET, with_param))
            // redirect
            .resource("/", |r| r.handler(Method::GET, |req, _, _| {
                println!("{:?}", req);

                httpcodes::HTTPFound
                    .builder()
                    .header("LOCATION", "/index.html")
                    .body(Body::Empty)
            }))
            // static files
            .route_handler("/static", StaticFiles::new("static/", true)))
        .serve::<_, ()>("127.0.0.1:8080").unwrap();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
