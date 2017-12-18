extern crate actix;
extern crate actix_web;
extern crate env_logger;
#[macro_use]
extern crate tera;
use actix_web::*;

struct State {
    template: tera::Tera,  // <- store tera template in application state
}

fn index(req: HttpRequest<State>) -> HttpResponse {
    let s = if let Some(name) = req.query().get("name") { // <- submitted form
        let mut ctx = tera::Context::new();
        ctx.add("name", name);
        ctx.add("text", &"Welcome!".to_owned());
        req.state().template.render("user.html", &ctx).unwrap()
    } else {
        req.state().template.render("index.html", &tera::Context::new()).unwrap()
    };
    httpcodes::HTTPOk.build()
        .content_type("text/html")
        .body(s)
        .unwrap()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("tera-example");

    HttpServer::new(|| {
        let tera = compile_templates!(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/**/*"));

        Application::with_state(State{template: tera})
            // enable logger
            .middleware(middlewares::Logger::default())
            .resource("/", |r| r.method(Method::GET).f(index))})
        .bind("127.0.0.1:8080").unwrap()
        .start().unwrap();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
