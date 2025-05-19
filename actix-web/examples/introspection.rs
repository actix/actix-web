// Example showcasing the experimental introspection feature.
// Run with: `cargo run --features experimental-introspection --example introspection`
use actix_web::{dev::Service, guard, web, App, HttpResponse, HttpServer, Responder};
use serde::Deserialize;

#[cfg(feature = "experimental-introspection")]
#[actix_web::get("/introspection")]
async fn introspection_handler() -> impl Responder {
    use std::fmt::Write;

    use actix_web::introspection::{get_registry, initialize_registry};

    initialize_registry();
    let registry = get_registry();
    let node = registry.lock().unwrap();

    let mut buf = String::new();
    if node.children.is_empty() {
        writeln!(buf, "No routes registered or introspection tree is empty.").unwrap();
    } else {
        fn write_display(
            node: &actix_web::introspection::IntrospectionNode,
            parent_path: &str,
            buf: &mut String,
        ) {
            let full_path = if parent_path.is_empty() {
                node.pattern.clone()
            } else {
                format!(
                    "{}/{}",
                    parent_path.trim_end_matches('/'),
                    node.pattern.trim_start_matches('/')
                )
            };
            if !node.methods.is_empty() || !node.guards.is_empty() {
                let methods = if node.methods.is_empty() {
                    "".to_string()
                } else {
                    format!("Methods: {:?}", node.methods)
                };

                let method_strings: Vec<String> =
                    node.methods.iter().map(|m| m.to_string()).collect();

                let filtered_guards: Vec<_> = node
                    .guards
                    .iter()
                    .filter(|guard| !method_strings.contains(&guard.to_string()))
                    .collect();

                let guards = if filtered_guards.is_empty() {
                    "".to_string()
                } else {
                    format!("Guards: {:?}", filtered_guards)
                };

                let _ = writeln!(buf, "{} {} {}", full_path, methods, guards);
            }
            for child in &node.children {
                write_display(child, &full_path, buf);
            }
        }
        write_display(&node, "/", &mut buf);
    }

    HttpResponse::Ok().content_type("text/plain").body(buf)
}

// Custom guard to check if the Content-Type header is present.
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
    // Initialize logging
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Debug)
        .init();

    let server = HttpServer::new(|| {
        let mut app = App::new()
            // API endpoints under /api
            .service(
                web::scope("/api")
                    // Endpoints under /api/v1
                    .service(
                        web::scope("/v1")
                            .service(get_item) // GET /api/v1/item/{id}
                            .service(post_user_info) // POST /api/v1/info
                            .route(
                                "/guarded",
                                web::route().guard(ContentTypeGuard).to(guarded_handler), // /api/v1/guarded
                            ),
                    )
                    // Endpoints under /api/v2
                    .service(web::scope("/v2").route("/hello", web::get().to(hello_v2))), // GET /api/v2/hello
            )
            // Endpoints under /v1 (outside /api)
            .service(web::scope("/v1").service(get_item)) // GET /v1/item/{id}
            // Admin endpoints under /admin
            .service(
                web::scope("/admin")
                    .route("/dashboard", web::get().to(admin_dashboard)) // GET /admin/dashboard
                    .service(
                        web::resource("/settings")
                            .route(web::get().to(get_settings)) // GET /admin/settings
                            .route(web::post().to(update_settings)), // POST /admin/settings
                    ),
            )
            // Root endpoints
            .service(
                web::resource("/")
                    .route(web::get().to(root_index)) // GET /
                    .route(web::post().to(root_index)), // POST /
            )
            // Endpoints under /bar
            .service(web::scope("/bar").configure(extra_endpoints)) // /bar/extra/ping, /bar/extra/multi, etc.
            // Endpoints under /foo
            .service(web::scope("/foo").configure(other_endpoints)) // /foo/extra/ping with POST and DELETE
            // Additional endpoints under /extra
            .configure(extra_endpoints) // /extra/ping, /extra/multi, etc.
            .configure(other_endpoints)
            // Endpoint that rejects GET on /not_guard (allows other methods)
            .route(
                "/not_guard",
                web::route()
                    .guard(guard::Not(guard::Get()))
                    .to(HttpResponse::MethodNotAllowed),
            )
            // Endpoint that requires GET with header or POST on /all_guard
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

        // Register the introspection handler if the feature is enabled.
        #[cfg(feature = "experimental-introspection")]
        {
            app = app.service(introspection_handler); // GET /introspection
        }
        app
    })
    .workers(1)
    .bind("127.0.0.1:8080")?;

    server.run().await
}

// GET /api/v1/item/{id} and GET /v1/item/{id}
#[actix_web::get("/item/{id}")]
async fn get_item(path: web::Path<u32>) -> impl Responder {
    let id = path.into_inner();
    HttpResponse::Ok().body(format!("Requested item with id: {}", id))
}

// POST /api/v1/info
#[actix_web::post("/info")]
async fn post_user_info(info: web::Json<UserInfo>) -> impl Responder {
    HttpResponse::Ok().json(format!(
        "User {} with age {} received",
        info.username, info.age
    ))
}

// /api/v1/guarded
async fn guarded_handler() -> impl Responder {
    HttpResponse::Ok().body("Passed the Content-Type guard!")
}

// GET /api/v2/hello
async fn hello_v2() -> impl Responder {
    HttpResponse::Ok().body("Hello from API v2!")
}

// GET /admin/dashboard
async fn admin_dashboard() -> impl Responder {
    HttpResponse::Ok().body("Welcome to the Admin Dashboard!")
}

// GET /admin/settings
async fn get_settings() -> impl Responder {
    HttpResponse::Ok().body("Current settings: ...")
}

// POST /admin/settings
async fn update_settings() -> impl Responder {
    HttpResponse::Ok().body("Settings have been updated!")
}

// GET and POST on /
async fn root_index() -> impl Responder {
    HttpResponse::Ok().body("Welcome to the Root Endpoint!")
}

// Additional endpoints for /extra
fn extra_endpoints(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/extra")
            .route(
                "/ping",
                web::get().to(|| async { HttpResponse::Ok().body("pong") }), // GET /extra/ping
            )
            .service(
                web::resource("/multi")
                    .route(
                        web::get().to(|| async {
                            HttpResponse::Ok().body("GET response from /extra/multi")
                        }),
                    ) // GET /extra/multi
                    .route(web::post().to(|| async {
                        HttpResponse::Ok().body("POST response from /extra/multi")
                    })), // POST /extra/multi
            )
            .service(
                web::scope("{entities_id:\\d+}")
                    .service(
                        web::scope("/secure")
                            .route(
                                "",
                                web::get().to(|| async {
                                    HttpResponse::Ok().body("GET response from /extra/secure")
                                }),
                            ) // GET /extra/{entities_id}/secure/
                            .route(
                                "/post",
                                web::post().to(|| async {
                                    HttpResponse::Ok().body("POST response from /extra/secure")
                                }),
                            ), // POST /extra/{entities_id}/secure/post
                    )
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

// Additional endpoints for /foo
fn other_endpoints(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/extra")
            .route(
                "/ping",
                web::post().to(|| async { HttpResponse::Ok().body("post from /extra/ping") }), // POST /foo/extra/ping
            )
            .route(
                "/ping",
                web::delete().to(|| async { HttpResponse::Ok().body("delete from /extra/ping") }), // DELETE /foo/extra/ping
            ),
    );
}
