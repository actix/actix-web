use std::{borrow::Cow, net::SocketAddr, rc::Rc};

use actix_http::{test::TestRequest as HttpTestRequest, Request};
use serde::Serialize;

#[cfg(feature = "cookies")]
use crate::cookie::{Cookie, CookieJar};
use crate::{
    app_service::AppInitServiceState,
    config::AppConfig,
    data::Data,
    dev::{Extensions, Path, Payload, ResourceDef, Service, Url},
    http::{
        header::{ContentType, TryIntoHeaderPair},
        Method, Uri, Version,
    },
    rmap::ResourceMap,
    service::{ServiceRequest, ServiceResponse},
    test,
    web::Bytes,
    HttpRequest, HttpResponse,
};

/// Test `Request` builder.
///
/// For unit testing, actix provides a request builder type and a simple handler runner. TestRequest implements a builder-like pattern.
/// You can generate various types of request via TestRequest's methods:
/// - [`TestRequest::to_request`] creates an [`actix_http::Request`](Request).
/// - [`TestRequest::to_srv_request`] creates a [`ServiceRequest`], which is used for testing middlewares and chain adapters.
/// - [`TestRequest::to_srv_response`] creates a [`ServiceResponse`].
/// - [`TestRequest::to_http_request`] creates an [`HttpRequest`], which is used for testing handlers.
///
/// ```
/// use actix_web::{test, HttpRequest, HttpResponse, HttpMessage};
/// use actix_web::http::{header, StatusCode};
///
/// async fn handler(req: HttpRequest) -> HttpResponse {
///     if let Some(hdr) = req.headers().get(header::CONTENT_TYPE) {
///         HttpResponse::Ok().into()
///     } else {
///         HttpResponse::BadRequest().into()
///     }
/// }
///
/// #[actix_web::test]
/// # // force rustdoc to display the correct thing and also compile check the test
/// # async fn _test() {}
/// async fn test_index() {
///     let req = test::TestRequest::default()
///         .insert_header(header::ContentType::plaintext())
///         .to_http_request();
///
///     let resp = handler(req).await;
///     assert_eq!(resp.status(), StatusCode::OK);
///
///     let req = test::TestRequest::default().to_http_request();
///     let resp = handler(req).await;
///     assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
/// }
/// ```
pub struct TestRequest {
    req: HttpTestRequest,
    rmap: ResourceMap,
    config: AppConfig,
    path: Path<Url>,
    peer_addr: Option<SocketAddr>,
    app_data: Extensions,
    #[cfg(feature = "cookies")]
    cookies: CookieJar,
}

impl Default for TestRequest {
    fn default() -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default(),
            rmap: ResourceMap::new(ResourceDef::new("")),
            config: AppConfig::default(),
            path: Path::new(Url::new(Uri::default())),
            peer_addr: None,
            app_data: Extensions::new(),
            #[cfg(feature = "cookies")]
            cookies: CookieJar::new(),
        }
    }
}

#[allow(clippy::wrong_self_convention)]
impl TestRequest {
    /// Constructs test request and sets request URI.
    pub fn with_uri(uri: &str) -> TestRequest {
        TestRequest::default().uri(uri)
    }

    /// Constructs test request with GET method.
    pub fn get() -> TestRequest {
        TestRequest::default().method(Method::GET)
    }

    /// Constructs test request with POST method.
    pub fn post() -> TestRequest {
        TestRequest::default().method(Method::POST)
    }

    /// Constructs test request with PUT method.
    pub fn put() -> TestRequest {
        TestRequest::default().method(Method::PUT)
    }

    /// Constructs test request with PATCH method.
    pub fn patch() -> TestRequest {
        TestRequest::default().method(Method::PATCH)
    }

    /// Constructs test request with DELETE method.
    pub fn delete() -> TestRequest {
        TestRequest::default().method(Method::DELETE)
    }

    /// Sets HTTP version of this request.
    pub fn version(mut self, ver: Version) -> Self {
        self.req.version(ver);
        self
    }

    /// Sets method of this request.
    pub fn method(mut self, meth: Method) -> Self {
        self.req.method(meth);
        self
    }

    /// Sets URI of this request.
    pub fn uri(mut self, path: &str) -> Self {
        self.req.uri(path);
        self
    }

    /// Inserts a header, replacing any that were set with an equivalent field name.
    pub fn insert_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        self.req.insert_header(header);
        self
    }

    /// Appends a header, keeping any that were set with an equivalent field name.
    pub fn append_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        self.req.append_header(header);
        self
    }

    /// Sets cookie for this request.
    #[cfg(feature = "cookies")]
    pub fn cookie(mut self, cookie: Cookie<'_>) -> Self {
        self.cookies.add(cookie.into_owned());
        self
    }

    /// Sets request path pattern parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// use actix_web::test::TestRequest;
    ///
    /// let req = TestRequest::default().param("foo", "bar");
    /// let req = TestRequest::default().param("foo".to_owned(), "bar".to_owned());
    /// ```
    pub fn param(
        mut self,
        name: impl Into<Cow<'static, str>>,
        value: impl Into<Cow<'static, str>>,
    ) -> Self {
        self.path.add_static(name, value);
        self
    }

    /// Sets peer address.
    pub fn peer_addr(mut self, addr: SocketAddr) -> Self {
        self.peer_addr = Some(addr);
        self
    }

    /// Sets request payload.
    pub fn set_payload(mut self, data: impl Into<Bytes>) -> Self {
        self.req.set_payload(data);
        self
    }

    /// Serializes `data` to a URL encoded form and set it as the request payload.
    ///
    /// The `Content-Type` header is set to `application/x-www-form-urlencoded`.
    pub fn set_form(mut self, data: impl Serialize) -> Self {
        let bytes = serde_urlencoded::to_string(&data)
            .expect("Failed to serialize test data as a urlencoded form");
        self.req.set_payload(bytes);
        self.req.insert_header(ContentType::form_url_encoded());
        self
    }

    /// Serializes `data` to JSON and set it as the request payload.
    ///
    /// The `Content-Type` header is set to `application/json`.
    pub fn set_json(mut self, data: impl Serialize) -> Self {
        let bytes = serde_json::to_string(&data).expect("Failed to serialize test data to json");
        self.req.set_payload(bytes);
        self.req.insert_header(ContentType::json());
        self
    }

    /// Inserts application data.
    ///
    /// This is equivalent of `App::app_data()` method for testing purpose.
    pub fn app_data<T: 'static>(mut self, data: T) -> Self {
        self.app_data.insert(data);
        self
    }

    /// Inserts application data.
    ///
    /// This is equivalent of `App::data()` method for testing purpose.
    #[doc(hidden)]
    pub fn data<T: 'static>(mut self, data: T) -> Self {
        self.app_data.insert(Data::new(data));
        self
    }

    /// Sets resource map.
    #[cfg(test)]
    pub(crate) fn rmap(mut self, rmap: ResourceMap) -> Self {
        self.rmap = rmap;
        self
    }

    /// Finalizes test request.
    ///
    /// This request builder will be useless after calling `finish()`.
    fn finish(&mut self) -> Request {
        // mut used when cookie feature is enabled
        #[allow(unused_mut)]
        let mut req = self.req.finish();

        #[cfg(feature = "cookies")]
        {
            use actix_http::header::{HeaderValue, COOKIE};

            let cookie: String = self
                .cookies
                .delta()
                // ensure only name=value is written to cookie header
                .map(|c| c.stripped().encoded().to_string())
                .collect::<Vec<_>>()
                .join("; ");

            if !cookie.is_empty() {
                req.headers_mut()
                    .insert(COOKIE, HeaderValue::from_str(&cookie).unwrap());
            }
        }

        req
    }

    /// Finalizes request creation and returns `Request` instance.
    pub fn to_request(mut self) -> Request {
        let mut req = self.finish();
        req.head_mut().peer_addr = self.peer_addr;
        req
    }

    /// Finalizes request creation and returns `ServiceRequest` instance.
    pub fn to_srv_request(mut self) -> ServiceRequest {
        let (mut head, payload) = self.finish().into_parts();
        head.peer_addr = self.peer_addr;
        self.path.get_mut().update(&head.uri);

        let app_state = AppInitServiceState::new(Rc::new(self.rmap), self.config.clone());

        ServiceRequest::new(
            HttpRequest::new(
                self.path,
                head,
                app_state,
                Rc::new(self.app_data),
                None,
                Default::default(),
            ),
            payload,
        )
    }

    /// Finalizes request creation and returns `ServiceResponse` instance.
    pub fn to_srv_response<B>(self, res: HttpResponse<B>) -> ServiceResponse<B> {
        self.to_srv_request().into_response(res)
    }

    /// Finalizes request creation and returns `HttpRequest` instance.
    pub fn to_http_request(mut self) -> HttpRequest {
        let (mut head, _) = self.finish().into_parts();
        head.peer_addr = self.peer_addr;
        self.path.get_mut().update(&head.uri);

        let app_state = AppInitServiceState::new(Rc::new(self.rmap), self.config.clone());

        HttpRequest::new(
            self.path,
            head,
            app_state,
            Rc::new(self.app_data),
            None,
            Default::default(),
        )
    }

    /// Finalizes request creation and returns `HttpRequest` and `Payload` pair.
    pub fn to_http_parts(mut self) -> (HttpRequest, Payload) {
        let (mut head, payload) = self.finish().into_parts();
        head.peer_addr = self.peer_addr;
        self.path.get_mut().update(&head.uri);

        let app_state = AppInitServiceState::new(Rc::new(self.rmap), self.config.clone());

        let req = HttpRequest::new(
            self.path,
            head,
            app_state,
            Rc::new(self.app_data),
            None,
            Default::default(),
        );

        (req, payload)
    }

    /// Finalizes request creation, calls service, and waits for response future completion.
    pub async fn send_request<S, B, E>(self, app: &S) -> S::Response
    where
        S: Service<Request, Response = ServiceResponse<B>, Error = E>,
        E: std::fmt::Debug,
    {
        let req = self.to_request();
        test::call_service(app, req).await
    }

    #[cfg(test)]
    pub fn set_server_hostname(&mut self, host: &str) {
        self.config.set_host(host)
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;
    use crate::{http::header, test::init_service, web, App, Error, Responder};

    #[actix_rt::test]
    async fn test_basics() {
        let req = TestRequest::default()
            .version(Version::HTTP_2)
            .insert_header(header::ContentType::json())
            .insert_header(header::Date(SystemTime::now().into()))
            .param("test", "123")
            .data(10u32)
            .app_data(20u64)
            .peer_addr("127.0.0.1:8081".parse().unwrap())
            .to_http_request();
        assert!(req.headers().contains_key(header::CONTENT_TYPE));
        assert!(req.headers().contains_key(header::DATE));
        assert_eq!(
            req.head().peer_addr,
            Some("127.0.0.1:8081".parse().unwrap())
        );
        assert_eq!(&req.match_info()["test"], "123");
        assert_eq!(req.version(), Version::HTTP_2);
        let data = req.app_data::<Data<u32>>().unwrap();
        assert!(req.app_data::<Data<u64>>().is_none());
        assert_eq!(*data.get_ref(), 10);

        assert!(req.app_data::<u32>().is_none());
        let data = req.app_data::<u64>().unwrap();
        assert_eq!(*data, 20);
    }

    #[actix_rt::test]
    async fn test_send_request() {
        let app = init_service(
            App::new().service(
                web::resource("/index.html")
                    .route(web::get().to(|| HttpResponse::Ok().body("welcome!"))),
            ),
        )
        .await;

        let resp = TestRequest::get()
            .uri("/index.html")
            .send_request(&app)
            .await;

        let result = test::read_body(resp).await;
        assert_eq!(result, Bytes::from_static(b"welcome!"));
    }

    #[actix_rt::test]
    async fn test_async_with_block() {
        async fn async_with_block() -> Result<HttpResponse, Error> {
            let res = web::block(move || Some(4usize).ok_or("wrong")).await;

            match res {
                Ok(value) => Ok(HttpResponse::Ok()
                    .content_type("text/plain")
                    .body(format!("Async with block value: {:?}", value))),
                Err(_) => panic!("Unexpected"),
            }
        }

        let app =
            init_service(App::new().service(web::resource("/index.html").to(async_with_block)))
                .await;

        let req = TestRequest::post().uri("/index.html").to_request();
        let res = app.call(req).await.unwrap();
        assert!(res.status().is_success());
    }

    // allow deprecated App::data
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_server_data() {
        async fn handler(data: web::Data<usize>) -> impl Responder {
            assert_eq!(**data, 10);
            HttpResponse::Ok()
        }

        let app = init_service(
            App::new()
                .data(10usize)
                .service(web::resource("/index.html").to(handler)),
        )
        .await;

        let req = TestRequest::post().uri("/index.html").to_request();
        let res = app.call(req).await.unwrap();
        assert!(res.status().is_success());
    }
}
