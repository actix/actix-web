// NOTE: This is a work-in-progress example being used to test the new implementation
// of the experimental introspection feature.
// `cargo run --features experimental-introspection --example introspection`

use actix_web::{dev::Service, guard, web, App, HttpResponse, HttpServer, Responder};
use serde::Deserialize;
// Custom guard that checks if the Content-Type header is present.
struct ContentTypeGuard;

impl guard::Guard for ContentTypeGuard {
    fn check(&self, req: &guard::GuardContext<'_>) -> bool {
        req.head()
            .headers()
            .contains_key(actix_web::http::header::CONTENT_TYPE)
    }
}

// Data structure for endpoints that receive JSON.
#[derive(Deserialize)]
struct UserInfo {
    username: String,
    age: u8,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let server = HttpServer::new(|| {
        let app = App::new()
            .service(
                web::scope("/api")
                    .service(
                        web::scope("/v1")
                            // GET /api/v1/item/{id}: returns the item id from the path.
                            .service(get_item)
                            // POST /api/v1/info: accepts JSON and returns user info.
                            .service(post_user_info)
                            // /api/v1/guarded: only accessible if Content-Type header is present.
                            .route(
                                "/guarded",
                                web::route().guard(ContentTypeGuard).to(guarded_handler),
                            ),
                    )
                    // API scope /api/v2: additional endpoint.
                    .service(web::scope("/v2").route("/hello", web::get().to(hello_v2))),
            )
            // Scope /v1 outside /api: exposes only GET /v1/item/{id}.
            .service(web::scope("/v1").service(get_item))
            // Scope /admin: admin endpoints with different HTTP methods.
            .service(
                web::scope("/admin")
                    .route("/dashboard", web::get().to(admin_dashboard))
                    // Single route handling multiple methods using separate handlers.
                    .service(
                        web::resource("/settings")
                            .route(web::get().to(get_settings))
                            .route(web::post().to(update_settings)),
                    ),
            )
            // Root resource: supports GET and POST on "/".
            .service(
                web::resource("/")
                    .route(web::get().to(root_index))
                    .route(web::post().to(root_index)),
            )
            // Additional endpoints configured in a separate function.
            .configure(extra_endpoints)
            // Endpoint that rejects GET on /not_guard (allows other methods).
            .route(
                "/not_guard",
                web::route()
                    .guard(guard::Not(guard::Get()))
                    .to(HttpResponse::MethodNotAllowed),
            )
            // Endpoint that requires GET, content-type: plain/text header, and/or POST on /all_guard.
            .route(
                "/all_guard",
                web::route()
                    .guard(
                        guard::All(guard::Get())
                            .and(guard::Header("content-type", "plain/text"))
                            .and(guard::Any(guard::Post())),
                    )
                    .to(HttpResponse::MethodNotAllowed),
            );

        /*#[cfg(feature = "experimental-introspection")]
        {
            actix_web::introspection::introspect();
        }*/
        // TODO: Enable introspection without the feature flag.
        app
    })
    .workers(5)
    .bind("127.0.0.1:8080")?;

    server.run().await
}

// GET /api/v1/item/{id} and GET /v1/item/{id}
// Returns a message with the provided id.
#[actix_web::get("/item/{id:\\d+}")]
async fn get_item(path: web::Path<u32>) -> impl Responder {
    let id = path.into_inner();
    HttpResponse::Ok().body(format!("Requested item with id: {}", id))
}

// POST /api/v1/info
// Expects JSON and responds with the received user info.
#[actix_web::post("/info")]
async fn post_user_info(info: web::Json<UserInfo>) -> impl Responder {
    HttpResponse::Ok().json(format!(
        "User {} with age {} received",
        info.username, info.age
    ))
}

// /api/v1/guarded
// Uses a custom guard that requires the Content-Type header.
async fn guarded_handler() -> impl Responder {
    HttpResponse::Ok().body("Passed the Content-Type guard!")
}

// GET /api/v2/hello
// Simple greeting endpoint.
async fn hello_v2() -> impl Responder {
    HttpResponse::Ok().body("Hello from API v2!")
}

// GET /admin/dashboard
// Returns a message for the admin dashboard.
async fn admin_dashboard() -> impl Responder {
    HttpResponse::Ok().body("Welcome to the Admin Dashboard!")
}

// GET /admin/settings
// Returns the current admin settings.
async fn get_settings() -> impl Responder {
    HttpResponse::Ok().body("Current settings: ...")
}

// POST /admin/settings
// Updates the admin settings.
async fn update_settings() -> impl Responder {
    HttpResponse::Ok().body("Settings have been updated!")
}

// GET and POST on /
// Generic root endpoint.
async fn root_index() -> impl Responder {
    HttpResponse::Ok().body("Welcome to the Root Endpoint!")
}

// Additional endpoints configured in a separate function.
fn extra_endpoints(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/extra")
            // GET /extra/ping: simple ping endpoint.
            .route(
                "/ping",
                web::get().to(|| async { HttpResponse::Ok().body("pong") }),
            )
            // /extra/multi: resource that supports GET and POST.
            .service(
                web::resource("/multi")
                    .route(
                        web::get().to(|| async {
                            HttpResponse::Ok().body("GET response from /extra/multi")
                        }),
                    )
                    .route(web::post().to(|| async {
                        HttpResponse::Ok().body("POST response from /extra/multi")
                    })),
            )
            // /extra/{entities_id}/secure: nested scope with GET and POST, prints the received id.
            .service(
                web::scope("{entities_id:\\d+}")
                    .service(
                        web::scope("/secure")
                            .route(
                                "",
                                web::get().to(|| async {
                                    HttpResponse::Ok().body("GET response from /extra/secure")
                                }),
                            )
                            .route(
                                "",
                                web::post().to(|| async {
                                    HttpResponse::Ok().body("POST response from /extra/secure")
                                }),
                            ),
                    )
                    // Middleware that prints the id received in the route.
                    .wrap_fn(|req, srv| {
                        println!(
                            "Request to /extra/secure with id: {}",
                            req.match_info().get("entities_id").unwrap()
                        );
                        let fut = srv.call(req);
                        async move {
                            let res = fut.await?;
                            Ok(res)
                        }
                    }),
            ),
    );
}
