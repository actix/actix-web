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
    if let Ok(ch) = req.payload_mut().readany() {
        if let futures::Async::Ready(Some(d)) = ch {
            println!("{}", String::from_utf8_lossy(d.0.as_ref()));
        }
    }

    // session
    if let Some(count) = req.session().get::<i32>("counter")? {
        println!("SESSION value: {}", count);
        req.session().set("counter", count+1)?;
    } else {
        req.session().set("counter", 1)?;
    }

    Ok(HttpResponse::Ok().into())
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
    let sys = actix::System::new("ws-example");

    HttpServer::new(
        Application::default("/")
            // enable logger
            .middleware(middlewares::Logger::default())
            // cookie session middleware
            .middleware(middlewares::SessionStorage::new(
                middlewares::CookieSessionBackend::build(&[0; 32])
                    .secure(false)
                    .finish()
            ))
            // register simple handle r, handle all methods
            .handler("/index.html", index)
            // with path parameters
            .resource("/user/{name}/", |r| r.handler(Method::GET, with_param))
            // async handler
            .resource("/async/{name}", |r| r.async(Method::GET, index_async))
            // redirect
            .resource("/", |r| r.handler(Method::GET, |req| {
                println!("{:?}", req);

                httpcodes::HTTPFound
                    .build()
                    .header("LOCATION", "/index.html")
                    .body(Body::Empty)
            }))
            .handler("/test", |req| {
                match *req.method() {
                    Method::GET => httpcodes::HTTPOk,
                    Method::POST => httpcodes::HTTPMethodNotAllowed,
                    _ => httpcodes::HTTPNotFound,
                }
            })
            // static files
            .route("/static", StaticFiles::new("examples/static/", true)))
        .serve::<_, ()>("127.0.0.1:8080").unwrap();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
