// Example showcasing the experimental introspection feature with multiple App instances.
// Run with: `cargo run --features experimental-introspection --example introspection_multi_servers`

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    #[cfg(feature = "experimental-introspection")]
    {
        use actix_web::{web, App, HttpResponse, HttpServer, Responder};
        use futures_util::future;

        async fn introspection_handler(
            tree: web::Data<actix_web::introspection::IntrospectionTree>,
        ) -> impl Responder {
            HttpResponse::Ok()
                .content_type("text/plain")
                .body(tree.report_as_text())
        }

        async fn index() -> impl Responder {
            HttpResponse::Ok().body("Hello from app")
        }

        let srv1 = HttpServer::new(|| {
            App::new()
                .service(web::resource("/a").route(web::get().to(index)))
                .service(
                    web::resource("/introspection").route(web::get().to(introspection_handler)),
                )
        })
        .workers(8)
        .bind("127.0.0.1:8081")?
        .run();

        let srv2 = HttpServer::new(|| {
            App::new()
                .service(web::resource("/b").route(web::get().to(index)))
                .service(
                    web::resource("/introspection").route(web::get().to(introspection_handler)),
                )
        })
        .workers(3)
        .bind("127.0.0.1:8082")?
        .run();

        future::try_join(srv1, srv2).await?;
    }
    #[cfg(not(feature = "experimental-introspection"))]
    {
        eprintln!("This example requires the 'experimental-introspection' feature to be enabled.");
    }
    Ok(())
}
