#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;

use std::fs::File;
use std::io::Read;

use actix::*;
use actix_web::*;
#[cfg(unix)]
use actix::actors::signal::{ProcessSignals, Subscribe};

/// somple handle
fn index(req: HttpRequest) -> Result<HttpResponse> {
    println!("{:?}", req);
    Ok(httpcodes::HTTPOk
       .build()
       .content_type("text/plain")
       .body("Welcome!")?)
}

fn main() {
    if ::std::env::var("RUST_LOG").is_err() {
        ::std::env::set_var("RUST_LOG", "actix_web=trace");
    }
    let _ = env_logger::init();
    let sys = actix::System::new("ws-example");

    let mut file = File::open("identity.pfx").unwrap();
    let mut pkcs12 = vec![];
    file.read_to_end(&mut pkcs12).unwrap();
    let pkcs12 = Pkcs12::from_der(&pkcs12).unwrap().parse("12345").unwrap();

    let addr = HttpServer::new(
        || Application::new()
            // enable logger
            .middleware(middleware::Logger::default())
            // register simple handler, handle all methods
            .resource("/index.html", |r| r.f(index))
            // with path parameters
            .resource("/", |r| r.method(Method::GET).f(|req| {
                httpcodes::HTTPFound
                    .build()
                    .header("LOCATION", "/index.html")
                    .body(Body::Empty)
            })))
        .bind("127.0.0.1:8443").unwrap()
        .start_ssl(&pkcs12).unwrap();

    // Subscribe to unix signals
    #[cfg(unix)]
    {
        let signals = Arbiter::system_registry().get::<ProcessSignals>();
        signals.send(Subscribe(addr.subscriber()));
    }

    println!("Started http server: 127.0.0.1:8443");
    let _ = sys.run();
}
