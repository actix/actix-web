use std::future::Future;

use actix_utils::future::{ok, Ready};
use actix_web::{
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    http::{
        self,
        header::{HeaderName, HeaderValue},
        StatusCode,
    },
    web, App, Error, HttpRequest, HttpResponse, Responder,
};
use actix_web_codegen::{
    connect, delete, get, head, options, patch, post, put, route, routes, trace,
};
use futures_core::future::LocalBoxFuture;

// Make sure that we can name function as 'config'
#[get("/config")]
async fn config() -> impl Responder {
    HttpResponse::Ok()
}

#[get("/test")]
async fn test_handler() -> impl Responder {
    HttpResponse::Ok()
}

#[put("/test")]
async fn put_test() -> impl Responder {
    HttpResponse::Created()
}

#[patch("/test")]
async fn patch_test() -> impl Responder {
    HttpResponse::Ok()
}

#[post("/test")]
async fn post_test() -> impl Responder {
    HttpResponse::NoContent()
}

#[head("/test")]
async fn head_test() -> impl Responder {
    HttpResponse::Ok()
}

#[connect("/test")]
async fn connect_test() -> impl Responder {
    HttpResponse::Ok()
}

#[options("/test")]
async fn options_test() -> impl Responder {
    HttpResponse::Ok()
}

#[trace("/test")]
async fn trace_test() -> impl Responder {
    HttpResponse::Ok()
}

#[get("/test")]
fn auto_async() -> impl Future<Output = Result<HttpResponse, actix_web::Error>> {
    ok(HttpResponse::Ok().finish())
}

#[get("/test")]
fn auto_sync() -> impl Future<Output = Result<HttpResponse, actix_web::Error>> {
    ok(HttpResponse::Ok().finish())
}

#[put("/test/{param}")]
async fn put_param_test(_: web::Path<String>) -> impl Responder {
    HttpResponse::Created()
}

#[delete("/test/{param}")]
async fn delete_param_test(_: web::Path<String>) -> impl Responder {
    HttpResponse::NoContent()
}

#[get("/test/{param}")]
async fn get_param_test(_: web::Path<String>) -> impl Responder {
    HttpResponse::Ok()
}

#[route("/hello", method = "HELLO")]
async fn custom_route_test() -> impl Responder {
    HttpResponse::Ok()
}

#[route(
    "/multi",
    method = "GET",
    method = "POST",
    method = "HEAD",
    method = "HELLO"
)]
async fn route_test() -> impl Responder {
    HttpResponse::Ok()
}

#[routes]
#[get("/routes/test")]
#[get("/routes/test2")]
#[post("/routes/test")]
async fn routes_test() -> impl Responder {
    HttpResponse::Ok()
}

// routes overlap with the more specific route first, therefore accessible
#[routes]
#[get("/routes/overlap/test")]
#[get("/routes/overlap/{foo}")]
async fn routes_overlapping_test(req: HttpRequest) -> impl Responder {
    // foo is only populated when route is not /routes/overlap/test
    match req.match_info().get("foo") {
        None => assert!(req.uri() == "/routes/overlap/test"),
        Some(_) => assert!(req.uri() != "/routes/overlap/test"),
    }

    HttpResponse::Ok()
}

// routes overlap with the more specific route last, therefore inaccessible
#[routes]
#[get("/routes/overlap2/{foo}")]
#[get("/routes/overlap2/test")]
async fn routes_overlapping_inaccessible_test(req: HttpRequest) -> impl Responder {
    // foo is always populated even when path is /routes/overlap2/test
    assert!(req.match_info().get("foo").is_some());

    HttpResponse::Ok()
}

#[get("/custom_resource_name", name = "custom")]
async fn custom_resource_name_test<'a>(req: HttpRequest) -> impl Responder {
    assert!(req.url_for_static("custom").is_ok());
    assert!(req.url_for_static("custom_resource_name_test").is_err());
    HttpResponse::Ok()
}

mod guard_module {
    use actix_web::{guard::GuardContext, http::header};

    pub fn guard(ctx: &GuardContext) -> bool {
        ctx.header::<header::Accept>()
            .map(|h| h.preference() == "image/*")
            .unwrap_or(false)
    }
}

#[get("/test/guard", guard = "guard_module::guard")]
async fn guard_test() -> impl Responder {
    HttpResponse::Ok()
}

pub struct ChangeStatusCode;

impl<S, B> Transform<S, ServiceRequest> for ChangeStatusCode
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = ChangeStatusCodeMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(ChangeStatusCodeMiddleware { service })
    }
}

pub struct ChangeStatusCodeMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for ChangeStatusCodeMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_web::dev::forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let fut = self.service.call(req);

        Box::pin(async move {
            let mut res = fut.await?;
            let headers = res.headers_mut();
            let header_name = HeaderName::from_lowercase(b"custom-header").unwrap();
            let header_value = HeaderValue::from_str("hello").unwrap();
            headers.insert(header_name, header_value);
            Ok(res)
        })
    }
}

#[get("/test/wrap", wrap = "ChangeStatusCode")]
async fn get_wrap(_: web::Path<String>) -> impl Responder {
    // panic!("actually never gets called because path failed to extract");
    HttpResponse::Ok()
}

/// Using expression, not just path to type, in wrap attribute.
///
/// Regression from <https://github.com/actix/actix-web/issues/3118>.
#[route(
    "/catalog",
    method = "GET",
    method = "HEAD",
    wrap = "actix_web::middleware::Compress::default()"
)]
async fn get_catalog() -> impl Responder {
    HttpResponse::Ok().body("123123123")
}

#[actix_rt::test]
async fn test_params() {
    let srv = actix_test::start(|| {
        App::new()
            .service(get_param_test)
            .service(put_param_test)
            .service(delete_param_test)
    });

    let request = srv.request(http::Method::GET, srv.url("/test/it"));
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::OK);

    let request = srv.request(http::Method::PUT, srv.url("/test/it"));
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::CREATED);

    let request = srv.request(http::Method::DELETE, srv.url("/test/it"));
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), http::StatusCode::NO_CONTENT);
}

#[actix_rt::test]
async fn test_body() {
    let srv = actix_test::start(|| {
        App::new()
            .service(post_test)
            .service(put_test)
            .service(head_test)
            .service(connect_test)
            .service(options_test)
            .service(trace_test)
            .service(patch_test)
            .service(test_handler)
            .service(route_test)
            .service(routes_overlapping_test)
            .service(routes_overlapping_inaccessible_test)
            .service(routes_test)
            .service(custom_resource_name_test)
            .service(guard_test)
    });
    let request = srv.request(http::Method::GET, srv.url("/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::HEAD, srv.url("/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::CONNECT, srv.url("/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::OPTIONS, srv.url("/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::TRACE, srv.url("/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::PATCH, srv.url("/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::PUT, srv.url("/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(response.status(), http::StatusCode::CREATED);

    let request = srv.request(http::Method::POST, srv.url("/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(response.status(), http::StatusCode::NO_CONTENT);

    let request = srv.request(http::Method::GET, srv.url("/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::GET, srv.url("/multi"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::POST, srv.url("/multi"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::HEAD, srv.url("/multi"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::PATCH, srv.url("/multi"));
    let response = request.send().await.unwrap();
    assert!(!response.status().is_success());

    let request = srv.request(http::Method::GET, srv.url("/routes/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::GET, srv.url("/routes/test2"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::POST, srv.url("/routes/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::GET, srv.url("/routes/not-set"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_client_error());

    let request = srv.request(http::Method::GET, srv.url("/routes/overlap/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::GET, srv.url("/routes/overlap/bar"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::GET, srv.url("/routes/overlap2/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::GET, srv.url("/routes/overlap2/bar"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv.request(http::Method::GET, srv.url("/custom_resource_name"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());

    let request = srv
        .request(http::Method::GET, srv.url("/test/guard"))
        .insert_header(("Accept", "image/*"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_rt::test]
async fn test_auto_async() {
    let srv = actix_test::start(|| App::new().service(auto_async));

    let request = srv.request(http::Method::GET, srv.url("/test"));
    let response = request.send().await.unwrap();
    assert!(response.status().is_success());
}

#[actix_web::test]
async fn test_wrap() {
    let srv = actix_test::start(|| App::new().service(get_wrap));

    let request = srv.request(http::Method::GET, srv.url("/test/wrap"));
    let mut response = request.send().await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.headers().contains_key("custom-header"));
    let body = response.body().await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("wrong number of parameters"));
}
