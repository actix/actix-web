extern crate actix;
extern crate actix_web;
extern crate env_logger;
#[macro_use]
extern crate tera;

use actix::*;
use actix_web::*;
#[cfg(target_os = "linux")] use actix::actors::signal::{ProcessSignals, Subscribe};

struct State {
    template: tera::Tera,  // <- store tera template in application state
}

fn index(req: HttpRequest<State>) -> Result<HttpResponse> {
    let s = if let Some(name) = req.query().get("name") { // <- submitted form
        let mut ctx = tera::Context::new();
        ctx.add("name", &name.to_owned());
        ctx.add("text", &"Welcome!".to_owned());
        req.state().template.render("user.html", &ctx)
            .map_err(|_| error::ErrorInternalServerError("Template error"))?
    } else {
        req.state().template.render("index.html", &tera::Context::new())
            .map_err(|_| error::ErrorInternalServerError("Template error"))?
    };
    Ok(httpcodes::HTTPOk.build()
       .content_type("text/html")
       .body(s)?)
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("tera-example");

    let addr = HttpServer::new(|| {
        let tera = compile_templates!(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/**/*"));

        Application::with_state(State{template: tera})
            // enable logger
            .middleware(middleware::Logger::default())
            .resource("/", |r| r.method(Method::GET).f(index))})
        .bind("127.0.0.1:8080").unwrap()
        .start();

    if cfg!(target_os = "linux") { // Subscribe to unix signals
        let signals = Arbiter::system_registry().get::<ProcessSignals>();
        signals.send(Subscribe(addr.subscriber()));
    }

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
