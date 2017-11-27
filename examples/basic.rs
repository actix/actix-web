#![allow(unused_variables)]
#![cfg_attr(feature="cargo-clippy", allow(needless_pass_by_value))]

extern crate actix;
extern crate actix_web;
extern crate env_logger;
extern crate futures;

use actix_web::*;
use actix_web::error::{Error, Result};
use actix_web::middlewares::RequestSession;
use futures::stream::{once, Once};

/// somple handle
fn index(mut req: HttpRequest, state: &()) -> Result<HttpResponse> {
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

    Ok(httpcodes::HTTPOk.into())
}

/// somple handle
fn index_async(req: HttpRequest, state: &()) -> Once<actix_web::Frame, Error>
{
    println!("{:?}", req);

    once(Ok(HttpResponse::builder(StatusCode::OK)
            .content_type("text/html")
            .body(format!("Hello {}!", req.match_info().get("name").unwrap()))
            .unwrap()
            .into()))
}

/// handle with path parameters like `/user/{name}/`
fn with_param(req: HttpRequest, state: &()) -> Result<HttpResponse>
{
    println!("{:?}", req);

    Ok(HttpResponse::builder(StatusCode::OK)
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
            .resource("/", |r| r.handler(Method::GET, |req, _| {
                println!("{:?}", req);

                Ok(httpcodes::HTTPFound
                   .builder()
                   .header("LOCATION", "/index.html")
                   .body(Body::Empty)?)
            }))
            // static files
            .route_handler("/static", StaticFiles::new("examples/static/", true)))
        .serve::<_, ()>("127.0.0.1:8080").unwrap();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
