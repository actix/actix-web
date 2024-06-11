use std::{
    cell::{Ref, RefCell, RefMut},
    fmt, net,
    rc::Rc,
    str,
};

use actix_http::{Message, RequestHead};
use actix_router::{Path, Url};
use actix_utils::future::{ok, Ready};
#[cfg(feature = "cookies")]
use cookie::{Cookie, ParseError as CookieParseError};
use smallvec::SmallVec;

use crate::{
    app_service::AppInitServiceState,
    config::AppConfig,
    dev::{Extensions, Payload},
    error::UrlGenerationError,
    http::{header::HeaderMap, Method, Uri, Version},
    info::ConnectionInfo,
    rmap::ResourceMap,
    Error, FromRequest, HttpMessage,
};

#[cfg(feature = "cookies")]
struct Cookies(Vec<Cookie<'static>>);

/// An incoming request.
#[derive(Clone)]
pub struct HttpRequest {
    /// # Invariant
    /// `Rc<HttpRequestInner>` is used exclusively and NO `Weak<HttpRequestInner>`
    /// is allowed anywhere in the code. Weak pointer is purposely ignored when
    /// doing `Rc`'s ref counter check. Expect panics if this invariant is violated.
    pub(crate) inner: Rc<HttpRequestInner>,
}

pub(crate) struct HttpRequestInner {
    pub(crate) head: Message<RequestHead>,
    pub(crate) path: Path<Url>,
    pub(crate) app_data: SmallVec<[Rc<Extensions>; 4]>,
    pub(crate) conn_data: Option<Rc<Extensions>>,
    pub(crate) extensions: Rc<RefCell<Extensions>>,
    app_state: Rc<AppInitServiceState>,
}

impl HttpRequest {
    #[inline]
    pub(crate) fn new(
        path: Path<Url>,
        head: Message<RequestHead>,
        app_state: Rc<AppInitServiceState>,
        app_data: Rc<Extensions>,
        conn_data: Option<Rc<Extensions>>,
        extensions: Rc<RefCell<Extensions>>,
    ) -> HttpRequest {
        let mut data = SmallVec::<[Rc<Extensions>; 4]>::new();
        data.push(app_data);

        HttpRequest {
            inner: Rc::new(HttpRequestInner {
                head,
                path,
                app_state,
                app_data: data,
                conn_data,
                extensions,
            }),
        }
    }
}

impl HttpRequest {
    /// This method returns reference to the request head
    #[inline]
    pub fn head(&self) -> &RequestHead {
        &self.inner.head
    }

    /// This method returns mutable reference to the request head.
    /// panics if multiple references of HTTP request exists.
    #[inline]
    pub(crate) fn head_mut(&mut self) -> &mut RequestHead {
        &mut Rc::get_mut(&mut self.inner).unwrap().head
    }

    /// Request's uri.
    #[inline]
    pub fn uri(&self) -> &Uri {
        &self.head().uri
    }

    /// Returns request's original full URL.
    ///
    /// Reconstructed URL is best-effort, using [`connection_info`](HttpRequest::connection_info())
    /// to get forwarded scheme & host.
    ///
    /// ```
    /// use actix_web::test::TestRequest;
    /// let req = TestRequest::with_uri("http://10.1.2.3:8443/api?id=4&name=foo")
    ///     .insert_header(("host", "example.com"))
    ///     .to_http_request();
    ///
    /// assert_eq!(
    ///     req.full_url().as_str(),
    ///     "http://example.com/api?id=4&name=foo",
    /// );
    /// ```
    pub fn full_url(&self) -> url::Url {
        let info = self.connection_info();
        let scheme = info.scheme();
        let host = info.host();
        let path_and_query = self
            .uri()
            .path_and_query()
            .map(|paq| paq.as_str())
            .unwrap_or("/");

        url::Url::parse(&format!("{scheme}://{host}{path_and_query}")).unwrap()
    }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.head().method
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.head().version
    }

    #[inline]
    /// Returns request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    /// The target path of this request.
    #[inline]
    pub fn path(&self) -> &str {
        self.head().uri.path()
    }

    /// The query string in the URL.
    ///
    /// Example: `id=10`
    #[inline]
    pub fn query_string(&self) -> &str {
        self.uri().query().unwrap_or_default()
    }

    /// Returns a reference to the URL parameters container.
    ///
    /// A URL parameter is specified in the form `{identifier}`, where the identifier can be used
    /// later in a request handler to access the matched value for that parameter.
    ///
    /// # Percent Encoding and URL Parameters
    /// Because each URL parameter is able to capture multiple path segments, none of
    /// `["%2F", "%25", "%2B"]` found in the request URI are decoded into `["/", "%", "+"]` in order
    /// to preserve path integrity. If a URL parameter is expected to contain these characters, then
    /// it is on the user to decode them or use the [`web::Path`](crate::web::Path) extractor which
    /// _will_ decode these special sequences.
    #[inline]
    pub fn match_info(&self) -> &Path<Url> {
        &self.inner.path
    }

    /// Returns a mutable reference to the URL parameters container.
    ///
    /// # Panics
    /// Panics if this `HttpRequest` has been cloned.
    #[inline]
    pub(crate) fn match_info_mut(&mut self) -> &mut Path<Url> {
        &mut Rc::get_mut(&mut self.inner).unwrap().path
    }

    /// The resource definition pattern that matched the path. Useful for logging and metrics.
    ///
    /// For example, when a resource with pattern `/user/{id}/profile` is defined and a call is made
    /// to `/user/123/profile` this function would return `Some("/user/{id}/profile")`.
    ///
    /// Returns a None when no resource is fully matched, including default services.
    #[inline]
    pub fn match_pattern(&self) -> Option<String> {
        self.resource_map().match_pattern(self.path())
    }

    /// The resource name that matched the path. Useful for logging and metrics.
    ///
    /// Returns a None when no resource is fully matched, including default services.
    #[inline]
    pub fn match_name(&self) -> Option<&str> {
        self.resource_map().match_name(self.path())
    }

    /// Returns a reference a piece of connection data set in an [on-connect] callback.
    ///
    /// ```ignore
    /// let opt_t = req.conn_data::<PeerCertificate>();
    /// ```
    ///
    /// [on-connect]: crate::HttpServer::on_connect
    pub fn conn_data<T: 'static>(&self) -> Option<&T> {
        self.inner
            .conn_data
            .as_deref()
            .and_then(|container| container.get::<T>())
    }

    /// Generates URL for a named resource.
    ///
    /// This substitutes in sequence all URL parameters that appear in the resource itself and in
    /// parent [scopes](crate::web::scope), if any.
    ///
    /// It is worth noting that the characters `['/', '%']` are not escaped and therefore a single
    /// URL parameter may expand into multiple path segments and `elements` can be percent-encoded
    /// beforehand without worrying about double encoding. Any other character that is not valid in
    /// a URL path context is escaped using percent-encoding.
    ///
    /// # Examples
    /// ```
    /// # use actix_web::{web, App, HttpRequest, HttpResponse};
    /// fn index(req: HttpRequest) -> HttpResponse {
    ///     let url = req.url_for("foo", &["1", "2", "3"]); // <- generate URL for "foo" resource
    ///     HttpResponse::Ok().into()
    /// }
    ///
    /// let app = App::new()
    ///     .service(web::resource("/test/{one}/{two}/{three}")
    ///          .name("foo")  // <- set resource name so it can be used in `url_for`
    ///          .route(web::get().to(|| HttpResponse::Ok()))
    ///     );
    /// ```
    pub fn url_for<U, I>(&self, name: &str, elements: U) -> Result<url::Url, UrlGenerationError>
    where
        U: IntoIterator<Item = I>,
        I: AsRef<str>,
    {
        self.resource_map().url_for(self, name, elements)
    }

    /// Generate URL for named resource
    ///
    /// This method is similar to `HttpRequest::url_for()` but it can be used
    /// for urls that do not contain variable parts.
    pub fn url_for_static(&self, name: &str) -> Result<url::Url, UrlGenerationError> {
        const NO_PARAMS: [&str; 0] = [];
        self.url_for(name, NO_PARAMS)
    }

    /// Get a reference to a `ResourceMap` of current application.
    #[inline]
    pub fn resource_map(&self) -> &ResourceMap {
        self.app_state().rmap()
    }

    /// Returns peer socket address.
    ///
    /// Peer address is the directly connected peer's socket address. If a proxy is used in front of
    /// the Actix Web server, then it would be address of this proxy.
    ///
    /// For expanded client connection information, use [`connection_info`] instead.
    ///
    /// Will only return None when called in unit tests unless [`TestRequest::peer_addr`] is used.
    ///
    /// [`TestRequest::peer_addr`]: crate::test::TestRequest::peer_addr
    /// [`connection_info`]: Self::connection_info
    #[inline]
    pub fn peer_addr(&self) -> Option<net::SocketAddr> {
        self.head().peer_addr
    }

    /// Returns connection info for the current request.
    ///
    /// The return type, [`ConnectionInfo`], can also be used as an extractor.
    ///
    /// # Panics
    /// Panics if request's extensions container is already borrowed.
    #[inline]
    pub fn connection_info(&self) -> Ref<'_, ConnectionInfo> {
        if !self.extensions().contains::<ConnectionInfo>() {
            let info = ConnectionInfo::new(self.head(), self.app_config());
            self.extensions_mut().insert(info);
        }

        Ref::map(self.extensions(), |data| data.get().unwrap())
    }

    /// Returns a reference to the application's connection configuration.
    #[inline]
    pub fn app_config(&self) -> &AppConfig {
        self.app_state().config()
    }

    /// Retrieves a piece of application state.
    ///
    /// Extracts any object stored with [`App::app_data()`](crate::App::app_data) (or the
    /// counterpart methods on [`Scope`](crate::Scope::app_data) and
    /// [`Resource`](crate::Resource::app_data)) during application configuration.
    ///
    /// Since the Actix Web router layers application data, the returned object will reference the
    /// "closest" instance of the type. For example, if an `App` stores a `u32`, a nested `Scope`
    /// also stores a `u32`, and the delegated request handler falls within that `Scope`, then
    /// calling `.app_data::<u32>()` on an `HttpRequest` within that handler will return the
    /// `Scope`'s instance. However, using the same router set up and a request that does not get
    /// captured by the `Scope`, `.app_data::<u32>()` would return the `App`'s instance.
    ///
    /// If the state was stored using the [`Data`] wrapper, then it must also be retrieved using
    /// this same type.
    ///
    /// See also the [`Data`] extractor.
    ///
    /// # Examples
    /// ```no_run
    /// # use actix_web::{test::TestRequest, web::Data};
    /// # let req = TestRequest::default().to_http_request();
    /// # type T = u32;
    /// let opt_t: Option<&Data<T>> = req.app_data::<Data<T>>();
    /// ```
    ///
    /// [`Data`]: crate::web::Data
    #[doc(alias = "state")]
    pub fn app_data<T: 'static>(&self) -> Option<&T> {
        for container in self.inner.app_data.iter().rev() {
            if let Some(data) = container.get::<T>() {
                return Some(data);
            }
        }

        None
    }

    #[inline]
    fn app_state(&self) -> &AppInitServiceState {
        &self.inner.app_state
    }

    /// Load request cookies.
    #[cfg(feature = "cookies")]
    pub fn cookies(&self) -> Result<Ref<'_, Vec<Cookie<'static>>>, CookieParseError> {
        use actix_http::header::COOKIE;

        if self.extensions().get::<Cookies>().is_none() {
            let mut cookies = Vec::new();
            for hdr in self.headers().get_all(COOKIE) {
                let s = str::from_utf8(hdr.as_bytes()).map_err(CookieParseError::from)?;
                for cookie_str in s.split(';').map(|s| s.trim()) {
                    if !cookie_str.is_empty() {
                        cookies.push(Cookie::parse_encoded(cookie_str)?.into_owned());
                    }
                }
            }
            self.extensions_mut().insert(Cookies(cookies));
        }

        Ok(Ref::map(self.extensions(), |ext| {
            &ext.get::<Cookies>().unwrap().0
        }))
    }

    /// Return request cookie.
    #[cfg(feature = "cookies")]
    pub fn cookie(&self, name: &str) -> Option<Cookie<'static>> {
        if let Ok(cookies) = self.cookies() {
            for cookie in cookies.iter() {
                if cookie.name() == name {
                    return Some(cookie.to_owned());
                }
            }
        }
        None
    }
}

impl HttpMessage for HttpRequest {
    type Stream = ();

    #[inline]
    fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    #[inline]
    fn extensions(&self) -> Ref<'_, Extensions> {
        self.inner.extensions.borrow()
    }

    #[inline]
    fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.inner.extensions.borrow_mut()
    }

    #[inline]
    fn take_payload(&mut self) -> Payload<Self::Stream> {
        Payload::None
    }
}

impl Drop for HttpRequest {
    fn drop(&mut self) {
        // if possible, contribute to current worker's HttpRequest allocation pool

        // This relies on no weak references to inner existing anywhere within the codebase.
        if let Some(inner) = Rc::get_mut(&mut self.inner) {
            if inner.app_state.pool().is_available() {
                // clear additional app_data and keep the root one for reuse.
                inner.app_data.truncate(1);

                // Inner is borrowed mut here and; get req data mutably to reduce borrow check. Also
                // we know the req_data Rc will not have any clones at this point to unwrap is okay.
                Rc::get_mut(&mut inner.extensions)
                    .unwrap()
                    .get_mut()
                    .clear();

                // We can't use the same trick as req data because the conn_data is held by the
                // dispatcher, too.
                inner.conn_data = None;

                // a re-borrow of pool is necessary here.
                let req = Rc::clone(&self.inner);
                self.app_state().pool().push(req);
            }
        }
    }
}

/// It is possible to get `HttpRequest` as an extractor handler parameter
///
/// # Examples
/// ```
/// use actix_web::{web, App, HttpRequest};
/// use serde::Deserialize;
///
/// /// extract `Thing` from request
/// async fn index(req: HttpRequest) -> String {
///    format!("Got thing: {:?}", req)
/// }
///
/// let app = App::new().service(
///     web::resource("/users/{first}").route(
///         web::get().to(index))
/// );
/// ```
impl FromRequest for HttpRequest {
    type Error = Error;
    type Future = Ready<Result<Self, Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ok(req.clone())
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "\nHttpRequest {:?} {}:{}",
            self.inner.head.version,
            self.inner.head.method,
            self.path()
        )?;

        if !self.query_string().is_empty() {
            writeln!(f, "  query: ?{:?}", self.query_string())?;
        }

        if !self.match_info().is_empty() {
            writeln!(f, "  params: {:?}", self.match_info())?;
        }

        writeln!(f, "  headers:")?;

        for (key, val) in self.headers().iter() {
            match key {
                // redact sensitive header values from debug output
                &crate::http::header::AUTHORIZATION
                | &crate::http::header::PROXY_AUTHORIZATION
                | &crate::http::header::COOKIE => writeln!(f, "    {:?}: {:?}", key, "*redacted*")?,

                _ => writeln!(f, "    {:?}: {:?}", key, val)?,
            }
        }

        Ok(())
    }
}

/// Slab-allocated `HttpRequest` Pool
///
/// Since request processing may yield for asynchronous events to complete, a worker may have many
/// requests in-flight at any time. Pooling requests like this amortizes the performance and memory
/// costs of allocating and de-allocating HttpRequest objects as frequently as they otherwise would.
///
/// Request objects are added when they are dropped (see `<HttpRequest as Drop>::drop`) and re-used
/// in `<AppInitService as Service>::call` when there are available objects in the list.
///
/// The pool's default capacity is 128 items.
pub(crate) struct HttpRequestPool {
    inner: RefCell<Vec<Rc<HttpRequestInner>>>,
    cap: usize,
}

impl Default for HttpRequestPool {
    fn default() -> Self {
        Self::with_capacity(128)
    }
}

impl HttpRequestPool {
    pub(crate) fn with_capacity(cap: usize) -> Self {
        HttpRequestPool {
            inner: RefCell::new(Vec::with_capacity(cap)),
            cap,
        }
    }

    /// Re-use a previously allocated (but now completed/discarded) HttpRequest object.
    #[inline]
    pub(crate) fn pop(&self) -> Option<HttpRequest> {
        self.inner
            .borrow_mut()
            .pop()
            .map(|inner| HttpRequest { inner })
    }

    /// Check if the pool still has capacity for request storage.
    #[inline]
    pub(crate) fn is_available(&self) -> bool {
        self.inner.borrow_mut().len() < self.cap
    }

    /// Push a request to pool.
    #[inline]
    pub(crate) fn push(&self, req: Rc<HttpRequestInner>) {
        self.inner.borrow_mut().push(req);
    }

    /// Clears all allocated HttpRequest objects.
    pub(crate) fn clear(&self) {
        self.inner.borrow_mut().clear()
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::{
        dev::{ResourceDef, Service},
        http::{header, StatusCode},
        test::{self, call_service, init_service, read_body, TestRequest},
        web, App, HttpResponse,
    };

    #[test]
    fn test_debug() {
        let req = TestRequest::default()
            .insert_header(("content-type", "text/plain"))
            .to_http_request();
        let dbg = format!("{:?}", req);
        assert!(dbg.contains("HttpRequest"));
    }

    #[test]
    #[cfg(feature = "cookies")]
    fn test_no_request_cookies() {
        let req = TestRequest::default().to_http_request();
        assert!(req.cookies().unwrap().is_empty());
    }

    #[test]
    #[cfg(feature = "cookies")]
    fn test_request_cookies() {
        let req = TestRequest::default()
            .append_header((header::COOKIE, "cookie1=value1"))
            .append_header((header::COOKIE, "cookie2=value2"))
            .to_http_request();
        {
            let cookies = req.cookies().unwrap();
            assert_eq!(cookies.len(), 2);
            assert_eq!(cookies[0].name(), "cookie1");
            assert_eq!(cookies[0].value(), "value1");
            assert_eq!(cookies[1].name(), "cookie2");
            assert_eq!(cookies[1].value(), "value2");
        }

        let cookie = req.cookie("cookie1");
        assert!(cookie.is_some());
        let cookie = cookie.unwrap();
        assert_eq!(cookie.name(), "cookie1");
        assert_eq!(cookie.value(), "value1");

        let cookie = req.cookie("cookie-unknown");
        assert!(cookie.is_none());
    }

    #[test]
    fn test_request_query() {
        let req = TestRequest::with_uri("/?id=test").to_http_request();
        assert_eq!(req.query_string(), "id=test");
    }

    #[test]
    fn test_url_for() {
        let mut res = ResourceDef::new("/user/{name}.{ext}");
        res.set_name("index");

        let mut rmap = ResourceMap::new(ResourceDef::prefix(""));
        rmap.add(&mut res, None);
        assert!(rmap.has_resource("/user/test.html"));
        assert!(!rmap.has_resource("/test/unknown"));

        let req = TestRequest::default()
            .insert_header((header::HOST, "www.rust-lang.org"))
            .rmap(rmap)
            .to_http_request();

        assert_eq!(
            req.url_for("unknown", ["test"]),
            Err(UrlGenerationError::ResourceNotFound)
        );
        assert_eq!(
            req.url_for("index", ["test"]),
            Err(UrlGenerationError::NotEnoughElements)
        );
        let url = req.url_for("index", ["test", "html"]);
        assert_eq!(
            url.ok().unwrap().as_str(),
            "http://www.rust-lang.org/user/test.html"
        );
    }

    #[test]
    fn test_url_for_static() {
        let mut rdef = ResourceDef::new("/index.html");
        rdef.set_name("index");

        let mut rmap = ResourceMap::new(ResourceDef::prefix(""));
        rmap.add(&mut rdef, None);

        assert!(rmap.has_resource("/index.html"));

        let req = TestRequest::with_uri("/test")
            .insert_header((header::HOST, "www.rust-lang.org"))
            .rmap(rmap)
            .to_http_request();
        let url = req.url_for_static("index");
        assert_eq!(
            url.ok().unwrap().as_str(),
            "http://www.rust-lang.org/index.html"
        );
    }

    #[test]
    fn test_match_name() {
        let mut rdef = ResourceDef::new("/index.html");
        rdef.set_name("index");

        let mut rmap = ResourceMap::new(ResourceDef::prefix(""));
        rmap.add(&mut rdef, None);

        assert!(rmap.has_resource("/index.html"));

        let req = TestRequest::default()
            .uri("/index.html")
            .rmap(rmap)
            .to_http_request();

        assert_eq!(req.match_name(), Some("index"));
    }

    #[test]
    fn test_url_for_external() {
        let mut rdef = ResourceDef::new("https://youtube.com/watch/{video_id}");

        rdef.set_name("youtube");

        let mut rmap = ResourceMap::new(ResourceDef::prefix(""));
        rmap.add(&mut rdef, None);

        let req = TestRequest::default().rmap(rmap).to_http_request();
        let url = req.url_for("youtube", ["oHg5SJYRHA0"]);
        assert_eq!(
            url.ok().unwrap().as_str(),
            "https://youtube.com/watch/oHg5SJYRHA0"
        );
    }

    #[actix_rt::test]
    async fn test_drop_http_request_pool() {
        let srv = init_service(
            App::new().service(web::resource("/").to(|req: HttpRequest| {
                HttpResponse::Ok()
                    .insert_header(("pool_cap", req.app_state().pool().cap))
                    .finish()
            })),
        )
        .await;

        let req = TestRequest::default().to_request();
        let resp = call_service(&srv, req).await;

        drop(srv);

        assert_eq!(resp.headers().get("pool_cap").unwrap(), "128");
    }

    #[actix_rt::test]
    async fn test_data() {
        let srv = init_service(App::new().app_data(10usize).service(web::resource("/").to(
            |req: HttpRequest| {
                if req.app_data::<usize>().is_some() {
                    HttpResponse::Ok()
                } else {
                    HttpResponse::BadRequest()
                }
            },
        )))
        .await;

        let req = TestRequest::default().to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let srv = init_service(App::new().app_data(10u32).service(web::resource("/").to(
            |req: HttpRequest| {
                if req.app_data::<usize>().is_some() {
                    HttpResponse::Ok()
                } else {
                    HttpResponse::BadRequest()
                }
            },
        )))
        .await;

        let req = TestRequest::default().to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn test_cascading_data() {
        #[allow(dead_code)]
        fn echo_usize(req: HttpRequest) -> HttpResponse {
            let num = req.app_data::<usize>().unwrap();
            HttpResponse::Ok().body(num.to_string())
        }

        let srv = init_service(
            App::new()
                .app_data(88usize)
                .service(web::resource("/").route(web::get().to(echo_usize)))
                .service(
                    web::resource("/one")
                        .app_data(1u32)
                        .route(web::get().to(echo_usize)),
                ),
        )
        .await;

        let req = TestRequest::get().uri("/").to_request();
        let resp = srv.call(req).await.unwrap();
        let body = read_body(resp).await;
        assert_eq!(body, Bytes::from_static(b"88"));

        let req = TestRequest::get().uri("/one").to_request();
        let resp = srv.call(req).await.unwrap();
        let body = read_body(resp).await;
        assert_eq!(body, Bytes::from_static(b"88"));
    }

    #[actix_rt::test]
    async fn test_overwrite_data() {
        #[allow(dead_code)]
        fn echo_usize(req: HttpRequest) -> HttpResponse {
            let num = req.app_data::<usize>().unwrap();
            HttpResponse::Ok().body(num.to_string())
        }

        let srv = init_service(
            App::new()
                .app_data(88usize)
                .service(web::resource("/").route(web::get().to(echo_usize)))
                .service(
                    web::resource("/one")
                        .app_data(1usize)
                        .route(web::get().to(echo_usize)),
                ),
        )
        .await;

        let req = TestRequest::get().uri("/").to_request();
        let resp = srv.call(req).await.unwrap();
        let body = read_body(resp).await;
        assert_eq!(body, Bytes::from_static(b"88"));

        let req = TestRequest::get().uri("/one").to_request();
        let resp = srv.call(req).await.unwrap();
        let body = read_body(resp).await;
        assert_eq!(body, Bytes::from_static(b"1"));
    }

    #[actix_rt::test]
    async fn test_app_data_dropped() {
        struct Tracker {
            pub dropped: bool,
        }
        struct Foo {
            tracker: Rc<RefCell<Tracker>>,
        }
        impl Drop for Foo {
            fn drop(&mut self) {
                self.tracker.borrow_mut().dropped = true;
            }
        }

        let tracker = Rc::new(RefCell::new(Tracker { dropped: false }));
        {
            let tracker2 = Rc::clone(&tracker);
            let srv = init_service(App::new().service(web::resource("/").to(
                move |req: HttpRequest| {
                    req.extensions_mut().insert(Foo {
                        tracker: Rc::clone(&tracker2),
                    });
                    HttpResponse::Ok()
                },
            )))
            .await;

            let req = TestRequest::default().to_request();
            let resp = call_service(&srv, req).await;
            assert_eq!(resp.status(), StatusCode::OK);
        }

        assert!(tracker.borrow().dropped);
    }

    #[actix_rt::test]
    async fn extract_path_pattern() {
        let srv = init_service(
            App::new().service(
                web::scope("/user/{id}")
                    .service(web::resource("/profile").route(web::get().to(
                        move |req: HttpRequest| {
                            assert_eq!(req.match_pattern(), Some("/user/{id}/profile".to_owned()));

                            HttpResponse::Ok().finish()
                        },
                    )))
                    .default_service(web::to(move |req: HttpRequest| {
                        assert!(req.match_pattern().is_none());
                        HttpResponse::Ok().finish()
                    })),
            ),
        )
        .await;

        let req = TestRequest::get().uri("/user/22/profile").to_request();
        let res = call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);

        let req = TestRequest::get().uri("/user/22/not-exist").to_request();
        let res = call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn extract_path_pattern_complex() {
        let srv = init_service(
            App::new()
                .service(web::scope("/user").service(web::scope("/{id}").service(
                    web::resource("").to(move |req: HttpRequest| {
                        assert_eq!(req.match_pattern(), Some("/user/{id}".to_owned()));

                        HttpResponse::Ok().finish()
                    }),
                )))
                .service(web::resource("/").to(move |req: HttpRequest| {
                    assert_eq!(req.match_pattern(), Some("/".to_owned()));

                    HttpResponse::Ok().finish()
                }))
                .default_service(web::to(move |req: HttpRequest| {
                    assert!(req.match_pattern().is_none());
                    HttpResponse::Ok().finish()
                })),
        )
        .await;

        let req = TestRequest::get().uri("/user/test").to_request();
        let res = call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);

        let req = TestRequest::get().uri("/").to_request();
        let res = call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);

        let req = TestRequest::get().uri("/not-exist").to_request();
        let res = call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn url_for_closest_named_resource() {
        // we mount the route named 'nested' on 2 different scopes, 'a' and 'b'
        let srv = test::init_service(
            App::new()
                .service(
                    web::scope("/foo")
                        .service(web::resource("/nested").name("nested").route(web::get().to(
                            |req: HttpRequest| {
                                HttpResponse::Ok()
                                    .body(format!("{}", req.url_for_static("nested").unwrap()))
                            },
                        )))
                        .service(web::scope("/baz").service(web::resource("deep")))
                        .service(web::resource("{foo_param}")),
                )
                .service(web::scope("/bar").service(
                    web::resource("/nested").name("nested").route(web::get().to(
                        |req: HttpRequest| {
                            HttpResponse::Ok()
                                .body(format!("{}", req.url_for_static("nested").unwrap()))
                        },
                    )),
                )),
        )
        .await;

        let foo_resp =
            test::call_service(&srv, TestRequest::with_uri("/foo/nested").to_request()).await;
        assert_eq!(foo_resp.status(), StatusCode::OK);
        let body = read_body(foo_resp).await;
        // `body` equals http://localhost:8080/bar/nested
        // because nested from /bar overrides /foo's
        // to do this any other way would require something like a custom tree search
        // see https://github.com/actix/actix-web/issues/1763
        assert_eq!(body, "http://localhost:8080/bar/nested");

        let bar_resp =
            test::call_service(&srv, TestRequest::with_uri("/bar/nested").to_request()).await;
        assert_eq!(bar_resp.status(), StatusCode::OK);
        let body = read_body(bar_resp).await;
        assert_eq!(body, "http://localhost:8080/bar/nested");
    }

    #[test]
    fn authorization_header_hidden_in_debug() {
        let authorization_header = "Basic bXkgdXNlcm5hbWU6bXkgcGFzc3dvcmQK";
        let req = TestRequest::get()
            .insert_header((crate::http::header::AUTHORIZATION, authorization_header))
            .to_http_request();

        assert!(!format!("{:?}", req).contains(authorization_header));
    }

    #[test]
    fn proxy_authorization_header_hidden_in_debug() {
        let proxy_authorization_header = "secret value";
        let req = TestRequest::get()
            .insert_header((
                crate::http::header::PROXY_AUTHORIZATION,
                proxy_authorization_header,
            ))
            .to_http_request();

        assert!(!format!("{:?}", req).contains(proxy_authorization_header));
    }

    #[test]
    fn cookie_header_hidden_in_debug() {
        let cookie_header = "secret";
        let req = TestRequest::get()
            .insert_header((crate::http::header::COOKIE, cookie_header))
            .to_http_request();

        assert!(!format!("{:?}", req).contains(cookie_header));
    }

    #[test]
    fn other_header_visible_in_debug() {
        let location_header = "192.0.0.1";
        let req = TestRequest::get()
            .insert_header((crate::http::header::LOCATION, location_header))
            .to_http_request();

        assert!(format!("{:?}", req).contains(location_header));
    }

    #[test]
    fn check_full_url() {
        let req = TestRequest::with_uri("/api?id=4&name=foo").to_http_request();
        assert_eq!(
            req.full_url().as_str(),
            "http://localhost:8080/api?id=4&name=foo",
        );

        let req = TestRequest::with_uri("https://example.com/api?id=4&name=foo").to_http_request();
        assert_eq!(
            req.full_url().as_str(),
            "https://example.com/api?id=4&name=foo",
        );

        let req = TestRequest::with_uri("http://10.1.2.3:8443/api?id=4&name=foo")
            .insert_header(("host", "example.com"))
            .to_http_request();
        assert_eq!(
            req.full_url().as_str(),
            "http://example.com/api?id=4&name=foo",
        );
    }
}
