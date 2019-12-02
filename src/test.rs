//! Various helpers for Actix applications to use during testing.
use std::rc::Rc;

use actix_http::http::header::{ContentType, Header, HeaderName, IntoHeaderValue};
use actix_http::http::{HttpTryFrom, Method, StatusCode, Uri, Version};
use actix_http::test::TestRequest as HttpTestRequest;
use actix_http::{cookie::Cookie, Extensions, Request};
use actix_router::{Path, ResourceDef, Url};
use actix_service::{IntoService, IntoServiceFactory, Service, ServiceFactory};
use bytes::{Bytes, BytesMut};
use futures::future::ok;
use futures::stream::{Stream, StreamExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json;

pub use actix_http::test::TestBuffer;

use crate::config::{AppConfig, AppConfigInner};
use crate::data::Data;
use crate::dev::{Body, MessageBody, Payload};
use crate::request::HttpRequestPool;
use crate::rmap::ResourceMap;
use crate::service::{ServiceRequest, ServiceResponse};
use crate::{Error, HttpRequest, HttpResponse};

/// Create service that always responds with `HttpResponse::Ok()`
pub fn ok_service(
) -> impl Service<Request = ServiceRequest, Response = ServiceResponse<Body>, Error = Error>
{
    default_service(StatusCode::OK)
}

/// Create service that responds with response with specified status code
pub fn default_service(
    status_code: StatusCode,
) -> impl Service<Request = ServiceRequest, Response = ServiceResponse<Body>, Error = Error>
{
    (move |req: ServiceRequest| {
        ok(req.into_response(HttpResponse::build(status_code).finish()))
    })
    .into_service()
}

/// This method accepts application builder instance, and constructs
/// service.
///
/// ```rust
/// use actix_service::Service;
/// use actix_web::{test, web, App, HttpResponse, http::StatusCode};
///
/// #[actix_rt::test]
/// async fn test_init_service() {
///     let mut app = test::init_service(
///         App::new()
///             .service(web::resource("/test").to(|| async { HttpResponse::Ok() }))
///     );
///
///     // Create request object
///     let req = test::TestRequest::with_uri("/test").to_request();
///
///     // Execute application
///     let resp = app.call(req).await.unwrap();
///     assert_eq!(resp.status(), StatusCode::OK);
/// }
/// ```
pub async fn init_service<R, S, B, E>(
    app: R,
) -> impl Service<Request = Request, Response = ServiceResponse<B>, Error = E>
where
    R: IntoServiceFactory<S>,
    S: ServiceFactory<
        Config = (),
        Request = Request,
        Response = ServiceResponse<B>,
        Error = E,
    >,
    S::InitError: std::fmt::Debug,
{
    let srv = app.into_factory();
    srv.new_service(()).await.unwrap()
}

/// Calls service and waits for response future completion.
///
/// ```rust
/// use actix_web::{test, App, HttpResponse, http::StatusCode};
/// use actix_service::Service;
///
/// #[test]
/// fn test_response() {
///     let mut app = test::init_service(
///         App::new()
///             .service(web::resource("/test").to(|| async {
///                 HttpResponse::Ok()
///             }))
///     ).await;
///
///     // Create request object
///     let req = test::TestRequest::with_uri("/test").to_request();
///
///     // Call application
///     let resp = test::call_service(&mut app, req).await;
///     assert_eq!(resp.status(), StatusCode::OK);
/// }
/// ```
pub async fn call_service<S, R, B, E>(app: &mut S, req: R) -> S::Response
where
    S: Service<Request = R, Response = ServiceResponse<B>, Error = E>,
    E: std::fmt::Debug,
{
    app.call(req).await.unwrap()
}

/// Helper function that returns a response body of a TestRequest
///
/// ```rust
/// use actix_web::{test, web, App, HttpResponse, http::header};
/// use bytes::Bytes;
///
/// #[actix_rt::test]
/// async fn test_index() {
///     let mut app = test::init_service(
///         App::new().service(
///             web::resource("/index.html")
///                 .route(web::post().to(|| async {
///                     HttpResponse::Ok().body("welcome!")
///                 })))
///     ).await;
///
///     let req = test::TestRequest::post()
///         .uri("/index.html")
///         .header(header::CONTENT_TYPE, "application/json")
///         .to_request();
///
///     let result = test::read_response(&mut app, req).await;
///     assert_eq!(result, Bytes::from_static(b"welcome!"));
/// }
/// ```
pub async fn read_response<S, B>(app: &mut S, req: Request) -> Bytes
where
    S: Service<Request = Request, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
{
    let mut resp = app
        .call(req)
        .await
        .unwrap_or_else(|_| panic!("read_response failed at application call"));

    let mut body = resp.take_body();
    let mut bytes = BytesMut::new();
    while let Some(item) = body.next().await {
        bytes.extend_from_slice(&item.unwrap());
    }
    bytes.freeze()
}

/// Helper function that returns a response body of a ServiceResponse.
///
/// ```rust
/// use actix_web::{test, web, App, HttpResponse, http::header};
/// use bytes::Bytes;
///
/// #[actix_rt::test]
/// async fn test_index() {
///     let mut app = test::init_service(
///         App::new().service(
///             web::resource("/index.html")
///                 .route(web::post().to(|| async {
///                     HttpResponse::Ok().body("welcome!")
///                 })))
///     ).await;
///
///     let req = test::TestRequest::post()
///         .uri("/index.html")
///         .header(header::CONTENT_TYPE, "application/json")
///         .to_request();
///
///     let resp = test::call_service(&mut app, req).await;
///     let result = test::read_body(resp);
///     assert_eq!(result, Bytes::from_static(b"welcome!"));
/// }
/// ```
pub async fn read_body<B>(mut res: ServiceResponse<B>) -> Bytes
where
    B: MessageBody,
{
    let mut body = res.take_body();
    let mut bytes = BytesMut::new();
    while let Some(item) = body.next().await {
        bytes.extend_from_slice(&item.unwrap());
    }
    bytes.freeze()
}

pub async fn load_stream<S>(mut stream: S) -> Result<Bytes, Error>
where
    S: Stream<Item = Result<Bytes, Error>> + Unpin,
{
    let mut data = BytesMut::new();
    while let Some(item) = stream.next().await {
        data.extend_from_slice(&item?);
    }
    Ok(data.freeze())
}

/// Helper function that returns a deserialized response body of a TestRequest
///
/// ```rust
/// use actix_web::{App, test, web, HttpResponse, http::header};
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize)]
/// pub struct Person {
///     id: String,
///     name: String
/// }
///
/// #[actix_rt::test]
/// async fn test_add_person() {
///     let mut app = test::init_service(
///         App::new().service(
///             web::resource("/people")
///                 .route(web::post().to(|person: web::Json<Person>| async {
///                     HttpResponse::Ok()
///                         .json(person.into_inner())})
///                     ))
///     ).await;
///
///     let payload = r#"{"id":"12345","name":"User name"}"#.as_bytes();
///
///     let req = test::TestRequest::post()
///         .uri("/people")
///         .header(header::CONTENT_TYPE, "application/json")
///         .set_payload(payload)
///         .to_request();
///
///     let result: Person = test::read_response_json(&mut app, req).await;
/// }
/// ```
pub async fn read_response_json<S, B, T>(app: &mut S, req: Request) -> T
where
    S: Service<Request = Request, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
    T: DeserializeOwned,
{
    let body = read_response(app, req).await;

    serde_json::from_slice(&body)
        .unwrap_or_else(|_| panic!("read_response_json failed during deserialization"))
}

/// Test `Request` builder.
///
/// For unit testing, actix provides a request builder type and a simple handler runner. TestRequest implements a builder-like pattern.
/// You can generate various types of request via TestRequest's methods:
///  * `TestRequest::to_request` creates `actix_http::Request` instance.
///  * `TestRequest::to_srv_request` creates `ServiceRequest` instance, which is used for testing middlewares and chain adapters.
///  * `TestRequest::to_srv_response` creates `ServiceResponse` instance.
///  * `TestRequest::to_http_request` creates `HttpRequest` instance, which is used for testing handlers.
///
/// ```rust
/// use actix_web::{test, HttpRequest, HttpResponse, HttpMessage};
/// use actix_web::http::{header, StatusCode};
///
/// async fn index(req: HttpRequest) -> HttpResponse {
///     if let Some(hdr) = req.headers().get(header::CONTENT_TYPE) {
///         HttpResponse::Ok().into()
///     } else {
///         HttpResponse::BadRequest().into()
///     }
/// }
///
/// #[test]
/// fn test_index() {
///     let req = test::TestRequest::with_header("content-type", "text/plain")
///         .to_http_request();
///
///     let resp = index(req).await.unwrap();
///     assert_eq!(resp.status(), StatusCode::OK);
///
///     let req = test::TestRequest::default().to_http_request();
///     let resp = index(req).await.unwrap();
///     assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
/// }
/// ```
pub struct TestRequest {
    req: HttpTestRequest,
    rmap: ResourceMap,
    config: AppConfigInner,
    path: Path<Url>,
    app_data: Extensions,
}

impl Default for TestRequest {
    fn default() -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default(),
            rmap: ResourceMap::new(ResourceDef::new("")),
            config: AppConfigInner::default(),
            path: Path::new(Url::new(Uri::default())),
            app_data: Extensions::new(),
        }
    }
}

#[allow(clippy::wrong_self_convention)]
impl TestRequest {
    /// Create TestRequest and set request uri
    pub fn with_uri(path: &str) -> TestRequest {
        TestRequest::default().uri(path)
    }

    /// Create TestRequest and set header
    pub fn with_hdr<H: Header>(hdr: H) -> TestRequest {
        TestRequest::default().set(hdr)
    }

    /// Create TestRequest and set header
    pub fn with_header<K, V>(key: K, value: V) -> TestRequest
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        TestRequest::default().header(key, value)
    }

    /// Create TestRequest and set method to `Method::GET`
    pub fn get() -> TestRequest {
        TestRequest::default().method(Method::GET)
    }

    /// Create TestRequest and set method to `Method::POST`
    pub fn post() -> TestRequest {
        TestRequest::default().method(Method::POST)
    }

    /// Create TestRequest and set method to `Method::PUT`
    pub fn put() -> TestRequest {
        TestRequest::default().method(Method::PUT)
    }

    /// Create TestRequest and set method to `Method::PATCH`
    pub fn patch() -> TestRequest {
        TestRequest::default().method(Method::PATCH)
    }

    /// Create TestRequest and set method to `Method::DELETE`
    pub fn delete() -> TestRequest {
        TestRequest::default().method(Method::DELETE)
    }

    /// Set HTTP version of this request
    pub fn version(mut self, ver: Version) -> Self {
        self.req.version(ver);
        self
    }

    /// Set HTTP method of this request
    pub fn method(mut self, meth: Method) -> Self {
        self.req.method(meth);
        self
    }

    /// Set HTTP Uri of this request
    pub fn uri(mut self, path: &str) -> Self {
        self.req.uri(path);
        self
    }

    /// Set a header
    pub fn set<H: Header>(mut self, hdr: H) -> Self {
        self.req.set(hdr);
        self
    }

    /// Set a header
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        self.req.header(key, value);
        self
    }

    /// Set cookie for this request
    pub fn cookie(mut self, cookie: Cookie) -> Self {
        self.req.cookie(cookie);
        self
    }

    /// Set request path pattern parameter
    pub fn param(mut self, name: &'static str, value: &'static str) -> Self {
        self.path.add_static(name, value);
        self
    }

    /// Set request payload
    pub fn set_payload<B: Into<Bytes>>(mut self, data: B) -> Self {
        self.req.set_payload(data);
        self
    }

    /// Serialize `data` to a URL encoded form and set it as the request payload. The `Content-Type`
    /// header is set to `application/x-www-form-urlencoded`.
    pub fn set_form<T: Serialize>(mut self, data: &T) -> Self {
        let bytes = serde_urlencoded::to_string(data)
            .expect("Failed to serialize test data as a urlencoded form");
        self.req.set_payload(bytes);
        self.req.set(ContentType::form_url_encoded());
        self
    }

    /// Serialize `data` to JSON and set it as the request payload. The `Content-Type` header is
    /// set to `application/json`.
    pub fn set_json<T: Serialize>(mut self, data: &T) -> Self {
        let bytes =
            serde_json::to_string(data).expect("Failed to serialize test data to json");
        self.req.set_payload(bytes);
        self.req.set(ContentType::json());
        self
    }

    /// Set application data. This is equivalent of `App::data()` method
    /// for testing purpose.
    pub fn data<T: 'static>(mut self, data: T) -> Self {
        self.app_data.insert(Data::new(data));
        self
    }

    #[cfg(test)]
    /// Set request config
    pub(crate) fn rmap(mut self, rmap: ResourceMap) -> Self {
        self.rmap = rmap;
        self
    }

    /// Complete request creation and generate `Request` instance
    pub fn to_request(mut self) -> Request {
        self.req.finish()
    }

    /// Complete request creation and generate `ServiceRequest` instance
    pub fn to_srv_request(mut self) -> ServiceRequest {
        let (head, payload) = self.req.finish().into_parts();
        self.path.get_mut().update(&head.uri);

        ServiceRequest::new(HttpRequest::new(
            self.path,
            head,
            payload,
            Rc::new(self.rmap),
            AppConfig::new(self.config),
            Rc::new(self.app_data),
            HttpRequestPool::create(),
        ))
    }

    /// Complete request creation and generate `ServiceResponse` instance
    pub fn to_srv_response<B>(self, res: HttpResponse<B>) -> ServiceResponse<B> {
        self.to_srv_request().into_response(res)
    }

    /// Complete request creation and generate `HttpRequest` instance
    pub fn to_http_request(mut self) -> HttpRequest {
        let (head, payload) = self.req.finish().into_parts();
        self.path.get_mut().update(&head.uri);

        HttpRequest::new(
            self.path,
            head,
            payload,
            Rc::new(self.rmap),
            AppConfig::new(self.config),
            Rc::new(self.app_data),
            HttpRequestPool::create(),
        )
    }

    /// Complete request creation and generate `HttpRequest` and `Payload` instances
    pub fn to_http_parts(mut self) -> (HttpRequest, Payload) {
        let (head, payload) = self.req.finish().into_parts();
        self.path.get_mut().update(&head.uri);

        let req = HttpRequest::new(
            self.path,
            head,
            Payload::None,
            Rc::new(self.rmap),
            AppConfig::new(self.config),
            Rc::new(self.app_data),
            HttpRequestPool::create(),
        );

        (req, payload)
    }
}

#[cfg(test)]
mod tests {
    use actix_http::httpmessage::HttpMessage;
    use serde::{Deserialize, Serialize};
    use std::time::SystemTime;

    use super::*;
    use crate::{http::header, web, App, HttpResponse};

    #[actix_rt::test]
    async fn test_basics() {
        let req = TestRequest::with_hdr(header::ContentType::json())
            .version(Version::HTTP_2)
            .set(header::Date(SystemTime::now().into()))
            .param("test", "123")
            .data(10u32)
            .to_http_request();
        assert!(req.headers().contains_key(header::CONTENT_TYPE));
        assert!(req.headers().contains_key(header::DATE));
        assert_eq!(&req.match_info()["test"], "123");
        assert_eq!(req.version(), Version::HTTP_2);
        let data = req.get_app_data::<u32>().unwrap();
        assert!(req.get_app_data::<u64>().is_none());
        assert_eq!(*data, 10);
        assert_eq!(*data.get_ref(), 10);

        assert!(req.app_data::<u64>().is_none());
        let data = req.app_data::<u32>().unwrap();
        assert_eq!(*data, 10);
    }

    #[actix_rt::test]
    async fn test_request_methods() {
        let mut app = init_service(
            App::new().service(
                web::resource("/index.html")
                    .route(web::put().to(|| async { HttpResponse::Ok().body("put!") }))
                    .route(
                        web::patch().to(|| async { HttpResponse::Ok().body("patch!") }),
                    )
                    .route(
                        web::delete()
                            .to(|| async { HttpResponse::Ok().body("delete!") }),
                    ),
            ),
        )
        .await;

        let put_req = TestRequest::put()
            .uri("/index.html")
            .header(header::CONTENT_TYPE, "application/json")
            .to_request();

        let result = read_response(&mut app, put_req).await;
        assert_eq!(result, Bytes::from_static(b"put!"));

        let patch_req = TestRequest::patch()
            .uri("/index.html")
            .header(header::CONTENT_TYPE, "application/json")
            .to_request();

        let result = read_response(&mut app, patch_req).await;
        assert_eq!(result, Bytes::from_static(b"patch!"));

        let delete_req = TestRequest::delete().uri("/index.html").to_request();
        let result = read_response(&mut app, delete_req).await;
        assert_eq!(result, Bytes::from_static(b"delete!"));
    }

    #[actix_rt::test]
    async fn test_response() {
        let mut app =
            init_service(App::new().service(web::resource("/index.html").route(
                web::post().to(|| async { HttpResponse::Ok().body("welcome!") }),
            )))
            .await;

        let req = TestRequest::post()
            .uri("/index.html")
            .header(header::CONTENT_TYPE, "application/json")
            .to_request();

        let result = read_response(&mut app, req).await;
        assert_eq!(result, Bytes::from_static(b"welcome!"));
    }

    #[derive(Serialize, Deserialize)]
    pub struct Person {
        id: String,
        name: String,
    }

    #[actix_rt::test]
    async fn test_response_json() {
        let mut app = init_service(App::new().service(web::resource("/people").route(
            web::post().to(|person: web::Json<Person>| {
                async { HttpResponse::Ok().json(person.into_inner()) }
            }),
        )))
        .await;

        let payload = r#"{"id":"12345","name":"User name"}"#.as_bytes();

        let req = TestRequest::post()
            .uri("/people")
            .header(header::CONTENT_TYPE, "application/json")
            .set_payload(payload)
            .to_request();

        let result: Person = read_response_json(&mut app, req).await;
        assert_eq!(&result.id, "12345");
    }

    #[actix_rt::test]
    async fn test_request_response_form() {
        let mut app = init_service(App::new().service(web::resource("/people").route(
            web::post().to(|person: web::Form<Person>| {
                async { HttpResponse::Ok().json(person.into_inner()) }
            }),
        )))
        .await;

        let payload = Person {
            id: "12345".to_string(),
            name: "User name".to_string(),
        };

        let req = TestRequest::post()
            .uri("/people")
            .set_form(&payload)
            .to_request();

        assert_eq!(req.content_type(), "application/x-www-form-urlencoded");

        let result: Person = read_response_json(&mut app, req).await;
        assert_eq!(&result.id, "12345");
        assert_eq!(&result.name, "User name");
    }

    #[actix_rt::test]
    async fn test_request_response_json() {
        let mut app = init_service(App::new().service(web::resource("/people").route(
            web::post().to(|person: web::Json<Person>| {
                async { HttpResponse::Ok().json(person.into_inner()) }
            }),
        )))
        .await;

        let payload = Person {
            id: "12345".to_string(),
            name: "User name".to_string(),
        };

        let req = TestRequest::post()
            .uri("/people")
            .set_json(&payload)
            .to_request();

        assert_eq!(req.content_type(), "application/json");

        let result: Person = read_response_json(&mut app, req).await;
        assert_eq!(&result.id, "12345");
        assert_eq!(&result.name, "User name");
    }

    #[actix_rt::test]
    async fn test_async_with_block() {
        async fn async_with_block() -> Result<HttpResponse, Error> {
            let res = web::block(move || Some(4usize).ok_or("wrong")).await;

            match res {
                Ok(value) => Ok(HttpResponse::Ok()
                    .content_type("text/plain")
                    .body(format!("Async with block value: {}", value))),
                Err(_) => panic!("Unexpected"),
            }
        }

        let mut app = init_service(
            App::new().service(web::resource("/index.html").to(async_with_block)),
        )
        .await;

        let req = TestRequest::post().uri("/index.html").to_request();
        let res = app.call(req).await.unwrap();
        assert!(res.status().is_success());
    }

    // #[actix_rt::test]
    // fn test_actor() {
    //     use actix::Actor;

    //     struct MyActor;

    //     struct Num(usize);
    //     impl actix::Message for Num {
    //         type Result = usize;
    //     }
    //     impl actix::Actor for MyActor {
    //         type Context = actix::Context<Self>;
    //     }
    //     impl actix::Handler<Num> for MyActor {
    //         type Result = usize;
    //         fn handle(&mut self, msg: Num, _: &mut Self::Context) -> Self::Result {
    //             msg.0
    //         }
    //     }

    //     let addr = run_on(|| MyActor.start());
    //     let mut app = init_service(App::new().service(
    //         web::resource("/index.html").to(move || {
    //             addr.send(Num(1)).from_err().and_then(|res| {
    //                 if res == 1 {
    //                     HttpResponse::Ok()
    //                 } else {
    //                     HttpResponse::BadRequest()
    //                 }
    //             })
    //         }),
    //     ));

    //     let req = TestRequest::post().uri("/index.html").to_request();
    //     let res = block_fn(|| app.call(req)).unwrap();
    //     assert!(res.status().is_success());
    // }
}
