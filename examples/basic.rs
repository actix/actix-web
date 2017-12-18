#![allow(unused_variables)]
#![cfg_attr(feature="cargo-clippy", allow(needless_pass_by_value))]

extern crate actix;
extern crate actix_web;
extern crate env_logger;
extern crate futures;

use actix_web::*;
use actix_web::middlewares::RequestSession;
use futures::future::{FutureResult, result};

/// simple handler
fn index(mut req: HttpRequest) -> Result<HttpResponse> {
    println!("{:?}", req);
    if let Some(payload) = req.payload_mut() {
        if let Ok(ch) = payload.readany() {
            if let futures::Async::Ready(Some(d)) = ch {
                println!("{}", String::from_utf8_lossy(d.0.as_ref()));
            }
        }
    }

    // session
    if let Some(count) = req.session().get::<i32>("counter")? {
        println!("SESSION value: {}", count);
        req.session().set("counter", count+1)?;
    } else {
        req.session().set("counter", 1)?;
    }

    Ok("Welcome!".into())
}

/// async handler
fn index_async(req: HttpRequest) -> FutureResult<HttpResponse, Error>
{
    println!("{:?}", req);

    result(HttpResponse::Ok()
           .content_type("text/html")
           .body(format!("Hello {}!", req.match_info().get("name").unwrap()))
           .map_err(|e| e.into()))
}

/// handler with path parameters like `/user/{name}/`
fn with_param(req: HttpRequest) -> Result<HttpResponse>
{
    println!("{:?}", req);

    Ok(HttpResponse::Ok()
       .content_type("test/plain")
       .body(format!("Hello {}!", req.match_info().get("name").unwrap()))?)
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("basic-example");

    HttpServer::new(
        || Application::new()
            // enable logger
            .middleware(middlewares::Logger::default())
            // cookie session middleware
            .middleware(middlewares::SessionStorage::new(
                middlewares::CookieSessionBackend::build(&[0; 32])
                    .secure(false)
                    .finish()
            ))
            // register simple route, handle all methods
            .resource("/index.html", |r| r.f(index))
            // with path parameters
            .resource("/user/{name}/", |r| r.method(Method::GET).f(with_param))
            // async handler
            .resource("/async/{name}", |r| r.method(Method::GET).a(index_async))
            .resource("/test", |r| r.f(|req| {
                match *req.method() {
                    Method::GET => httpcodes::HTTPOk,
                    Method::POST => httpcodes::HTTPMethodNotAllowed,
                    _ => httpcodes::HTTPNotFound,
                }
            }))
            // static files
            .resource("/static/{tail:.*}",
                      |r| r.h(fs::StaticFiles::new("tail", "examples/static/", true)))
            // redirect
            .resource("/", |r| r.method(Method::GET).f(|req| {
                println!("{:?}", req);

                httpcodes::HTTPFound
                    .build()
                    .header("LOCATION", "/index.html")
                    .body(Body::Empty)
            })))
        .bind("127.0.0.1:8080").unwrap()
        .start().unwrap();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
