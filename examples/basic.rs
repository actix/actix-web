use futures::IntoFuture;

use actix_web::{
    http::Method, middleware, web, App, Error, HttpRequest, HttpResponse, HttpServer,
    Resource, Route,
};

fn index(req: HttpRequest) -> &'static str {
    println!("REQ: {:?}", req);
    "Hello world!\r\n"
}

fn index_async(req: HttpRequest) -> impl IntoFuture<Item = &'static str, Error = Error> {
    println!("REQ: {:?}", req);
    Ok("Hello world!\r\n")
}

fn no_params() -> &'static str {
    "Hello world!\r\n"
}

fn main() -> std::io::Result<()> {
    ::std::env::set_var("RUST_LOG", "actix_server=info,actix_web2=info");
    env_logger::init();
    let sys = actix_rt::System::new("hello-world");

    HttpServer::new(|| {
        App::new()
            .middleware(middleware::DefaultHeaders::new().header("X-Version", "0.2"))
            .middleware(middleware::Compress::default())
            .resource("/resource1/index.html", |r| r.route(web::get().to(index)))
            .service(
                "/resource2/index.html",
                Resource::new()
                    .middleware(
                        middleware::DefaultHeaders::new().header("X-Version-R2", "0.3"),
                    )
                    .default_resource(|r| {
                        r.route(Route::new().to(|| HttpResponse::MethodNotAllowed()))
                    })
                    .route(web::method(Method::GET).to_async(index_async)),
            )
            .service("/test1.html", Resource::new().to(|| "Test\r\n"))
            .service("/", Resource::new().to(no_params))
    })
    .bind("127.0.0.1:8080")?
    .workers(1)
    .start();

    let _ = sys.run();
    Ok(())
}
