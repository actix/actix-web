#![cfg(feature = "experimental-introspection")]

use actix_web::{guard, test, web, App, HttpResponse};

async fn introspection_handler(
    tree: web::Data<actix_web::introspection::IntrospectionTree>,
) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("application/json")
        .body(tree.report_as_json())
}

async fn externals_handler(
    tree: web::Data<actix_web::introspection::IntrospectionTree>,
) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("application/json")
        .body(tree.report_externals_as_json())
}

fn find_item<'a>(items: &'a [serde_json::Value], path: &str) -> &'a serde_json::Value {
    items
        .iter()
        .find(|item| item.get("full_path").and_then(|v| v.as_str()) == Some(path))
        .unwrap_or_else(|| panic!("missing route for {path}"))
}

fn find_external<'a>(items: &'a [serde_json::Value], name: &str) -> &'a serde_json::Value {
    items
        .iter()
        .find(|item| item.get("name").and_then(|v| v.as_str()) == Some(name))
        .unwrap_or_else(|| panic!("missing external resource for {name}"))
}

#[actix_rt::test]
async fn introspection_report_includes_details_and_metadata() {
    let app = test::init_service(
        App::new()
            .external_resource("app-external", "https://example.com/{id}")
            .service(
                web::resource(["/alpha", "/beta"])
                    .name("multi")
                    .route(web::get().to(HttpResponse::Ok)),
            )
            .service(
                web::resource("/guarded")
                    .guard(guard::Header("accept", "text/plain"))
                    .route(web::get().to(HttpResponse::Ok)),
            )
            .service(
                web::scope("/scoped")
                    .guard(guard::Header("x-scope", "1"))
                    .configure(|cfg| {
                        cfg.external_resource("scope-external", "https://scope.example/{id}");
                    })
                    .service(web::resource("/item").route(web::get().to(HttpResponse::Ok))),
            )
            .service(web::resource("/introspection").route(web::get().to(introspection_handler)))
            .service(
                web::resource("/introspection/externals").route(web::get().to(externals_handler)),
            ),
    )
    .await;

    let req = test::TestRequest::get().uri("/introspection").to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let items: Vec<serde_json::Value> =
        serde_json::from_slice(&body).expect("invalid introspection json");

    let alpha = find_item(&items, "/alpha");
    let patterns = alpha
        .get("patterns")
        .and_then(|v| v.as_array())
        .expect("patterns missing");
    let patterns = patterns
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>();
    assert!(patterns.contains(&"/alpha"));
    assert!(patterns.contains(&"/beta"));
    assert_eq!(
        alpha.get("resource_name").and_then(|v| v.as_str()),
        Some("multi")
    );
    assert_eq!(
        alpha.get("resource_type").and_then(|v| v.as_str()),
        Some("resource")
    );

    let guarded = find_item(&items, "/guarded");
    let guards = guarded
        .get("guards")
        .and_then(|v| v.as_array())
        .expect("guards missing");
    assert!(guards
        .iter()
        .any(|v| v.as_str() == Some("Header(accept, text/plain)")));

    let guard_details = guarded
        .get("guards_detail")
        .and_then(|v| v.as_array())
        .expect("guards_detail missing");
    assert!(!guard_details.is_empty());

    let alpha_guards = alpha
        .get("guards")
        .and_then(|v| v.as_array())
        .expect("alpha guards missing");
    let alpha_guard_details = alpha
        .get("guards_detail")
        .and_then(|v| v.as_array())
        .expect("alpha guards_detail missing");
    assert!(alpha_guards.is_empty());
    assert!(!alpha_guard_details.is_empty());

    let scoped = find_item(&items, "/scoped");
    assert_eq!(
        scoped.get("resource_type").and_then(|v| v.as_str()),
        Some("scope")
    );
    let scoped_guards = scoped
        .get("guards")
        .and_then(|v| v.as_array())
        .expect("scoped guards missing");
    assert!(scoped_guards
        .iter()
        .any(|v| v.as_str() == Some("Header(x-scope, 1)")));

    let req = test::TestRequest::get()
        .uri("/introspection/externals")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let externals: Vec<serde_json::Value> =
        serde_json::from_slice(&body).expect("invalid externals json");

    let app_external = find_external(&externals, "app-external");
    let app_patterns = app_external
        .get("patterns")
        .and_then(|v| v.as_array())
        .expect("app external patterns missing");
    assert!(app_patterns
        .iter()
        .any(|v| v.as_str() == Some("https://example.com/{id}")));
    assert_eq!(
        app_external.get("origin_scope").and_then(|v| v.as_str()),
        Some("/")
    );

    let scope_external = find_external(&externals, "scope-external");
    let scope_patterns = scope_external
        .get("patterns")
        .and_then(|v| v.as_array())
        .expect("scope external patterns missing");
    assert!(scope_patterns
        .iter()
        .any(|v| v.as_str() == Some("https://scope.example/{id}")));
    assert_eq!(
        scope_external.get("origin_scope").and_then(|v| v.as_str()),
        Some("/scoped")
    );
}
