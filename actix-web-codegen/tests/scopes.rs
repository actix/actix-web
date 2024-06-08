use actix_web::{guard::GuardContext, http, http::header, web, App, HttpResponse, Responder};
use actix_web_codegen::{delete, get, post, route, routes, scope};

pub fn image_guard(ctx: &GuardContext) -> bool {
    ctx.header::<header::Accept>()
        .map(|h| h.preference() == "image/*")
        .unwrap_or(false)
}

#[scope("/test")]
mod scope_module {
    // ensure that imports can be brought into the scope
    use super::*;

    #[get("/test/guard", guard = "image_guard")]
    pub async fn guard() -> impl Responder {
        HttpResponse::Ok()
    }

    #[get("/test")]
    pub async fn test() -> impl Responder {
        HttpResponse::Ok().finish()
    }

    #[get("/twice-test/{value}")]
    pub async fn twice(value: web::Path<String>) -> impl actix_web::Responder {
        let int_value: i32 = value.parse().unwrap_or(0);
        let doubled = int_value * 2;
        HttpResponse::Ok().body(format!("Twice value: {}", doubled))
    }

    #[post("/test")]
    pub async fn post() -> impl Responder {
        HttpResponse::Ok().body("post works")
    }

    #[delete("/test")]
    pub async fn delete() -> impl Responder {
        "delete works"
    }

    #[route("/test", method = "PUT", method = "PATCH", method = "CUSTOM")]
    pub async fn multiple_shared_path() -> impl Responder {
        HttpResponse::Ok().finish()
    }

    #[routes]
    #[head("/test1")]
    #[connect("/test2")]
    #[options("/test3")]
    #[trace("/test4")]
    pub async fn multiple_separate_paths() -> impl Responder {
        HttpResponse::Ok().finish()
    }

    // test calling this from other mod scope with scope attribute...
    pub fn mod_common(message: String) -> impl actix_web::Responder {
        HttpResponse::Ok().body(message)
    }
}

/// Scope doc string to check in cargo expand.
#[scope("/v1")]
mod mod_scope_v1 {
    use super::*;

    /// Route doc string to check in cargo expand.
    #[get("/test")]
    pub async fn test() -> impl Responder {
        scope_module::mod_common("version1 works".to_string())
    }
}

#[scope("/v2")]
mod mod_scope_v2 {
    use super::*;

    // check to make sure non-function tokens in the scope block are preserved...
    enum TestEnum {
        Works,
    }

    #[get("/test")]
    pub async fn test() -> impl Responder {
        // make sure this type still exists...
        let test_enum = TestEnum::Works;

        match test_enum {
            TestEnum::Works => scope_module::mod_common("version2 works".to_string()),
        }
    }
}

#[actix_rt::test]
async fn scope_get_async() {
    let srv = actix_test::start(|| App::new().service(scope_module::test));

    let request = srv.request(http::Method::GET, srv.url("/test/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn scope_get_param_async() {
    let srv = actix_test::start(|| App::new().service(scope_module::twice));

    let request = srv.request(http::Method::GET, srv.url("/test/twice-test/4"));
    let mut response = request.send().await.unwrap();
    let body = response.body().await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(body_str, "Twice value: 8");
}

#[actix_rt::test]
async fn scope_post_async() {
    let srv = actix_test::start(|| App::new().service(scope_module::post));

    let request = srv.request(http::Method::POST, srv.url("/test/test"));
    let mut response = request.send().await.unwrap();
    let body = response.body().await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(body_str, "post works");
}

#[actix_rt::test]
async fn multiple_shared_path_async() {
    let srv = actix_test::start(|| App::new().service(scope_module::multiple_shared_path));

    let request = srv.request(http::Method::PUT, srv.url("/test/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::PATCH, srv.url("/test/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn multiple_multi_path_async() {
    let srv = actix_test::start(|| App::new().service(scope_module::multiple_separate_paths));

    let request = srv.request(http::Method::HEAD, srv.url("/test/test1"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::CONNECT, srv.url("/test/test2"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::OPTIONS, srv.url("/test/test3"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::TRACE, srv.url("/test/test4"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn scope_delete_async() {
    let srv = actix_test::start(|| App::new().service(scope_module::delete));

    let request = srv.request(http::Method::DELETE, srv.url("/test/test"));
    let mut response = request.send().await.unwrap();
    let body = response.body().await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(body_str, "delete works");
}

#[actix_rt::test]
async fn scope_get_with_guard_async() {
    let srv = actix_test::start(|| App::new().service(scope_module::guard));

    let request = srv
        .request(http::Method::GET, srv.url("/test/test/guard"))
        .insert_header(("Accept", "image/*"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn scope_v1_v2_async() {
    let srv = actix_test::start(|| {
        App::new()
            .service(mod_scope_v1::test)
            .service(mod_scope_v2::test)
    });

    let request = srv.request(http::Method::GET, srv.url("/v1/test"));
    let mut response = request.send().await.unwrap();
    let body = response.body().await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(body_str, "version1 works");

    let request = srv.request(http::Method::GET, srv.url("/v2/test"));
    let mut response = request.send().await.unwrap();
    let body = response.body().await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(body_str, "version2 works");
}
