extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix_web::*;


fn index(_req: HttpRequest) -> &'static str {
    "hello, world"
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("catflap-example");

    let fd = std::env::var("LISTEN_FD").ok()
        .and_then(|fd| fd.parse().ok())
        .expect("couldn't get LISTEN_FD env variable");

    let _addr = HttpServer::new(
        || Application::new()
            .resource("/", |r| r.f(index)))
        .bind_socket(fd).unwrap()
        .start();

    println!("Started http server.");
    let _ = sys.run();
}
