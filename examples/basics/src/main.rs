#![allow(unused_variables)]
#![cfg_attr(feature="cargo-clippy", allow(needless_pass_by_value))]

extern crate actix;
extern crate actix_web;
extern crate env_logger;
extern crate futures;
use futures::Stream;

use std::{io, env};
use actix_web::*;
use actix_web::middleware::RequestSession;
use futures::future::{FutureResult, result};

/// favicon handler
fn favicon(req: HttpRequest) -> Result<fs::NamedFile> {
    Ok(fs::NamedFile::open("../static/favicon.ico")?)
}

/// simple index handler
fn index(mut req: HttpRequest) -> Result<HttpResponse> {
    println!("{:?}", req);

    // example of ...
    if let Ok(ch) = req.poll() {
        if let futures::Async::Ready(Some(d)) = ch {
            println!("{}", String::from_utf8_lossy(d.as_ref()));
        }
    }

    // session
    let mut counter = 1;
    if let Some(count) = req.session().get::<i32>("counter")? {
        println!("SESSION value: {}", count);
        counter = count + 1;
        req.session().set("counter", counter)?;
    } else {
        req.session().set("counter", counter)?;
    }

    // html
    let html = format!(r#"<!DOCTYPE html><html><head><title>actix - basics</title><link rel="shortcut icon" type="image/x-icon" href="/favicon.ico" /></head>
<body>
    <h1>Welcome <img width="30px" height="30px" src="/static/actixLogo.png" /></h1>
    session counter = {}
</body>
</html>"#, counter);

    // response
    Ok(HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(&html).unwrap())

}

/// 404 handler
fn p404(req: HttpRequest) -> Result<HttpResponse> {

    // html
    let html = r#"<!DOCTYPE html><html><head><title>actix - basics</title><link rel="shortcut icon" type="image/x-icon" href="/favicon.ico" /></head>
<body>
    <a href="index.html">back to home</a>
    <h1>404</h1>
</body>
</html>"#;

    // response
    Ok(HttpResponse::build(StatusCode::NOT_FOUND)
        .content_type("text/html; charset=utf-8")
        .body(html).unwrap())
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
    env::set_var("RUST_LOG", "actix_web=debug");
    env::set_var("RUST_BACKTRACE", "1");
    env_logger::init();
    let sys = actix::System::new("basic-example");

    let addr = HttpServer::new(
        || Application::new()
            // enable logger
            .middleware(middleware::Logger::default())
            // cookie session middleware
            .middleware(middleware::SessionStorage::new(
                middleware::CookieSessionBackend::build(&[0; 32])
                    .secure(false)
                    .finish()
            ))
            // register favicon
            .resource("/favicon.ico", |r| r.f(favicon))
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
            .resource("/error.html", |r| r.f(|req| {
                error::InternalError::new(
                    io::Error::new(io::ErrorKind::Other, "test"), StatusCode::OK)
            }))
            // static files
            .handler("/static/", fs::StaticFiles::new("../static/", true))
            // redirect
            .resource("/", |r| r.method(Method::GET).f(|req| {
                println!("{:?}", req);

                HttpResponse::Found()
                    .header("LOCATION", "/index.html")
                    .finish()
            }))
            // default
            .default_resource(|r| {
                r.method(Method::GET).f(p404);
                r.route().filter(pred::Not(pred::Get())).f(|req| httpcodes::HTTPMethodNotAllowed);
            }))

        .bind("127.0.0.1:8080").expect("Can not bind to 127.0.0.1:8080")
        .shutdown_timeout(0)    // <- Set shutdown timeout to 0 seconds (default 60s)
        .start();

    println!("Starting http server: 127.0.0.1:8080");
    let _ = sys.run();
}
