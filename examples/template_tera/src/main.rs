extern crate actix;
extern crate actix_web;
extern crate env_logger;
#[macro_use]
extern crate tera;

use actix_web::{
    http, error, middleware, server, App, HttpRequest, HttpResponse, Error};


struct State {
    template: tera::Tera,  // <- store tera template in application state
}

fn index(req: HttpRequest<State>) -> Result<HttpResponse, Error> {
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
    Ok(HttpResponse::Ok()
       .content_type("text/html")
       .body(s))
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();
    let sys = actix::System::new("tera-example");

    server::new(|| {
        let tera = compile_templates!(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/**/*"));

        App::with_state(State{template: tera})
            // enable logger
            .middleware(middleware::Logger::default())
            .resource("/", |r| r.method(http::Method::GET).f(index))})
        .bind("127.0.0.1:8080").unwrap()
        .start();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
