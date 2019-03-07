//! Various helpers for Actix applications to use during testing.
use std::rc::Rc;
use std::str::FromStr;
use std::sync::mpsc;
use std::{net, thread};

use actix::{Actor, Addr, System};
use actix::actors::signal;

use actix_net::server::Server;
use cookie::Cookie;
use futures::Future;
use http::header::HeaderName;
use http::{HeaderMap, HttpTryFrom, Method, Uri, Version};
use net2::TcpBuilder;
use tokio::runtime::current_thread::Runtime;

#[cfg(any(feature = "alpn", feature = "ssl"))]
use openssl::ssl::SslAcceptorBuilder;
#[cfg(feature = "rust-tls")]
use rustls::ServerConfig;

use application::{App, HttpApplication};
use body::Binary;
use client::{ClientConnector, ClientRequest, ClientRequestBuilder};
use error::Error;
use handler::{AsyncResult, AsyncResultItem, Handler, Responder};
use header::{Header, IntoHeaderValue};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::Middleware;
use param::Params;
use payload::Payload;
use resource::Resource;
use router::Router;
use server::message::{Request, RequestPool};
use server::{HttpServer, IntoHttpHandler, ServerSettings};
use uri::Url as InnerUrl;
use ws;

/// The `TestServer` type.
///
/// `TestServer` is very simple test server that simplify process of writing
/// integration tests cases for actix web applications.
///
/// # Examples
///
/// ```rust
/// # extern crate actix_web;
/// # use actix_web::*;
/// #
/// # fn my_handler(req: &HttpRequest) -> HttpResponse {
/// #     HttpResponse::Ok().into()
/// # }
/// #
/// # fn main() {
/// use actix_web::test::TestServer;
///
/// let mut srv = TestServer::new(|app| app.handler(my_handler));
///
/// let req = srv.get().finish().unwrap();
/// let response = srv.execute(req.send()).unwrap();
/// assert!(response.status().is_success());
/// # }
/// ```
pub struct TestServer {
    addr: net::SocketAddr,
    ssl: bool,
    conn: Addr<ClientConnector>,
    rt: Runtime,
    backend: Addr<Server>,
}

impl TestServer {
    /// Start new test server
    ///
    /// This method accepts configuration method. You can add
    /// middlewares or set handlers for test application.
    pub fn new<F>(config: F) -> Self
    where
        F: Clone + Send + 'static + Fn(&mut TestApp<()>),
    {
        TestServerBuilder::new(|| ()).start(config)
    }

    /// Create test server builder
    pub fn build() -> TestServerBuilder<(), impl Fn() -> () + Clone + Send + 'static> {
        TestServerBuilder::new(|| ())
    }

    /// Create test server builder with specific state factory
    ///
    /// This method can be used for constructing application state.
    /// Also it can be used for external dependency initialization,
    /// like creating sync actors for diesel integration.
    pub fn build_with_state<S, F>(state: F) -> TestServerBuilder<S, F>
    where
        F: Fn() -> S + Clone + Send + 'static,
        S: 'static,
    {
        TestServerBuilder::new(state)
    }

    /// Start new test server with application factory
    pub fn with_factory<F, H>(factory: F) -> Self
    where
        F: Fn() -> H + Send + Clone + 'static,
        H: IntoHttpHandler + 'static,
    {
        let (tx, rx) = mpsc::channel();

        // run server in separate thread
        thread::spawn(move || {
            let sys = System::new("actix-test-server");
            let tcp = net::TcpListener::bind("127.0.0.1:0").unwrap();
            let local_addr = tcp.local_addr().unwrap();

            let srv = HttpServer::new(factory)
                .disable_signals()
                .listen(tcp)
                .keep_alive(5)
                .start();

            tx.send((System::current(), local_addr, TestServer::get_conn(), srv))
                .unwrap();
            sys.run();
        });

        let (system, addr, conn, backend) = rx.recv().unwrap();
        System::set_current(system);
        TestServer {
            addr,
            conn,
            ssl: false,
            rt: Runtime::new().unwrap(),
            backend,
        }
    }

    fn get_conn() -> Addr<ClientConnector> {
        #[cfg(any(feature = "alpn", feature = "ssl"))]
        {
            use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

            let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
            builder.set_verify(SslVerifyMode::NONE);
            ClientConnector::with_connector(builder.build()).start()
        }
        #[cfg(all(
            feature = "rust-tls",
            not(any(feature = "alpn", feature = "ssl"))
        ))]
        {
            use rustls::ClientConfig;
            use std::fs::File;
            use std::io::BufReader;
            let mut config = ClientConfig::new();
            let pem_file = &mut BufReader::new(File::open("tests/cert.pem").unwrap());
            config.root_store.add_pem_file(pem_file).unwrap();
            ClientConnector::with_connector(config).start()
        }
        #[cfg(not(any(feature = "alpn", feature = "ssl", feature = "rust-tls")))]
        {
            ClientConnector::default().start()
        }
    }

    /// Get firat available unused address
    pub fn unused_addr() -> net::SocketAddr {
        let addr: net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let socket = TcpBuilder::new_v4().unwrap();
        socket.bind(&addr).unwrap();
        socket.reuse_address(true).unwrap();
        let tcp = socket.to_tcp_listener().unwrap();
        tcp.local_addr().unwrap()
    }

    /// Construct test server url
    pub fn addr(&self) -> net::SocketAddr {
        self.addr
    }

    /// Construct test server url
    pub fn url(&self, uri: &str) -> String {
        if uri.starts_with('/') {
            format!(
                "{}://localhost:{}{}",
                if self.ssl { "https" } else { "http" },
                self.addr.port(),
                uri
            )
        } else {
            format!(
                "{}://localhost:{}/{}",
                if self.ssl { "https" } else { "http" },
                self.addr.port(),
                uri
            )
        }
    }

    /// Stop http server
    fn stop(&mut self) {
        let _ = self.backend.send(signal::Signal(signal::SignalType::Term)).wait();
        System::current().stop();
    }

    /// Execute future on current core
    pub fn execute<F, I, E>(&mut self, fut: F) -> Result<I, E>
    where
        F: Future<Item = I, Error = E>,
    {
        self.rt.block_on(fut)
    }

    /// Connect to websocket server at a given path
    pub fn ws_at(
        &mut self, path: &str,
    ) -> Result<(ws::ClientReader, ws::ClientWriter), ws::ClientError> {
        let url = self.url(path);
        self.rt
            .block_on(ws::Client::with_connector(url, self.conn.clone()).connect())
    }

    /// Connect to a websocket server
    pub fn ws(
        &mut self,
    ) -> Result<(ws::ClientReader, ws::ClientWriter), ws::ClientError> {
        self.ws_at("/")
    }

    /// Create `GET` request
    pub fn get(&self) -> ClientRequestBuilder {
        ClientRequest::get(self.url("/").as_str())
    }

    /// Create `POST` request
    pub fn post(&self) -> ClientRequestBuilder {
        ClientRequest::post(self.url("/").as_str())
    }

    /// Create `PATCH` request
    pub fn patch(&self) -> ClientRequestBuilder {
        ClientRequest::patch(self.url("/").as_str())
    }

    /// Create `HEAD` request
    pub fn head(&self) -> ClientRequestBuilder {
        ClientRequest::head(self.url("/").as_str())
    }

    /// Connect to test http server
    pub fn client(&self, meth: Method, path: &str) -> ClientRequestBuilder {
        ClientRequest::build()
            .method(meth)
            .uri(self.url(path).as_str())
            .with_connector(self.conn.clone())
            .take()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop()
    }
}

/// An `TestServer` builder
///
/// This type can be used to construct an instance of `TestServer` through a
/// builder-like pattern.
pub struct TestServerBuilder<S, F>
where
    F: Fn() -> S + Send + Clone + 'static,
{
    state: F,
    #[cfg(any(feature = "alpn", feature = "ssl"))]
    ssl: Option<SslAcceptorBuilder>,
    #[cfg(feature = "rust-tls")]
    rust_ssl: Option<ServerConfig>,
}

impl<S: 'static, F> TestServerBuilder<S, F>
where
    F: Fn() -> S + Send + Clone + 'static,
{
    /// Create a new test server
    pub fn new(state: F) -> TestServerBuilder<S, F> {
        TestServerBuilder {
            state,
            #[cfg(any(feature = "alpn", feature = "ssl"))]
            ssl: None,
            #[cfg(feature = "rust-tls")]
            rust_ssl: None,
        }
    }

    #[cfg(any(feature = "alpn", feature = "ssl"))]
    /// Create ssl server
    pub fn ssl(mut self, ssl: SslAcceptorBuilder) -> Self {
        self.ssl = Some(ssl);
        self
    }

    #[cfg(feature = "rust-tls")]
    /// Create rust tls server
    pub fn rustls(mut self, ssl: ServerConfig) -> Self {
        self.rust_ssl = Some(ssl);
        self
    }

    #[allow(unused_mut)]
    /// Configure test application and run test server
    pub fn start<C>(mut self, config: C) -> TestServer
    where
        C: Fn(&mut TestApp<S>) + Clone + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();

        let mut has_ssl = false;

        #[cfg(any(feature = "alpn", feature = "ssl"))]
        {
            has_ssl = has_ssl || self.ssl.is_some();
        }

        #[cfg(feature = "rust-tls")]
        {
            has_ssl = has_ssl || self.rust_ssl.is_some();
        }

        // run server in separate thread
        thread::spawn(move || {
            let addr = TestServer::unused_addr();

            let sys = System::new("actix-test-server");
            let state = self.state;
            let mut srv = HttpServer::new(move || {
                let mut app = TestApp::new(state());
                config(&mut app);
                app
            }).workers(1)
            .keep_alive(5)
            .disable_signals();



            #[cfg(any(feature = "alpn", feature = "ssl"))]
            {
                let ssl = self.ssl.take();
                if let Some(ssl) = ssl {
                    let tcp = net::TcpListener::bind(addr).unwrap();
                    srv = srv.listen_ssl(tcp, ssl).unwrap();
                }
            }
            #[cfg(feature = "rust-tls")]
            {
                let ssl = self.rust_ssl.take();
                if let Some(ssl) = ssl {
                    let tcp = net::TcpListener::bind(addr).unwrap();
                    srv = srv.listen_rustls(tcp, ssl);
                }
            }
            if !has_ssl {
                let tcp = net::TcpListener::bind(addr).unwrap();
                srv = srv.listen(tcp);
            }
            let backend = srv.start();

            tx.send((System::current(), addr, TestServer::get_conn(), backend))
                .unwrap();

            sys.run();
        });

        let (system, addr, conn, backend) = rx.recv().unwrap();
        System::set_current(system);
        TestServer {
            addr,
            conn,
            ssl: has_ssl,
            rt: Runtime::new().unwrap(),
            backend,
        }
    }
}

/// Test application helper for testing request handlers.
pub struct TestApp<S = ()> {
    app: Option<App<S>>,
}

impl<S: 'static> TestApp<S> {
    fn new(state: S) -> TestApp<S> {
        let app = App::with_state(state);
        TestApp { app: Some(app) }
    }

    /// Register handler for "/"
    pub fn handler<F, R>(&mut self, handler: F)
    where
        F: Fn(&HttpRequest<S>) -> R + 'static,
        R: Responder + 'static,
    {
        self.app = Some(self.app.take().unwrap().resource("/", |r| r.f(handler)));
    }

    /// Register middleware
    pub fn middleware<T>(&mut self, mw: T) -> &mut TestApp<S>
    where
        T: Middleware<S> + 'static,
    {
        self.app = Some(self.app.take().unwrap().middleware(mw));
        self
    }

    /// Register resource. This method is similar
    /// to `App::resource()` method.
    pub fn resource<F, R>(&mut self, path: &str, f: F) -> &mut TestApp<S>
    where
        F: FnOnce(&mut Resource<S>) -> R + 'static,
    {
        self.app = Some(self.app.take().unwrap().resource(path, f));
        self
    }
}

impl<S: 'static> IntoHttpHandler for TestApp<S> {
    type Handler = HttpApplication<S>;

    fn into_handler(mut self) -> HttpApplication<S> {
        self.app.take().unwrap().into_handler()
    }
}

#[doc(hidden)]
impl<S: 'static> Iterator for TestApp<S> {
    type Item = HttpApplication<S>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(mut app) = self.app.take() {
            Some(app.finish())
        } else {
            None
        }
    }
}

/// Test `HttpRequest` builder
///
/// ```rust
/// # extern crate http;
/// # extern crate actix_web;
/// # use http::{header, StatusCode};
/// # use actix_web::*;
/// use actix_web::test::TestRequest;
///
/// fn index(req: &HttpRequest) -> HttpResponse {
///     if let Some(hdr) = req.headers().get(header::CONTENT_TYPE) {
///         HttpResponse::Ok().into()
///     } else {
///         HttpResponse::BadRequest().into()
///     }
/// }
///
/// fn main() {
///     let resp = TestRequest::with_header("content-type", "text/plain")
///         .run(&index)
///         .unwrap();
///     assert_eq!(resp.status(), StatusCode::OK);
///
///     let resp = TestRequest::default().run(&index).unwrap();
///     assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
/// }
/// ```
pub struct TestRequest<S> {
    state: S,
    version: Version,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    params: Params,
    cookies: Option<Vec<Cookie<'static>>>,
    payload: Option<Payload>,
    prefix: u16,
}

impl Default for TestRequest<()> {
    fn default() -> TestRequest<()> {
        TestRequest {
            state: (),
            method: Method::GET,
            uri: Uri::from_str("/").unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            params: Params::new(),
            cookies: None,
            payload: None,
            prefix: 0,
        }
    }
}

impl TestRequest<()> {
    /// Create TestRequest and set request uri
    pub fn with_uri(path: &str) -> TestRequest<()> {
        TestRequest::default().uri(path)
    }

    /// Create TestRequest and set header
    pub fn with_hdr<H: Header>(hdr: H) -> TestRequest<()> {
        TestRequest::default().set(hdr)
    }

    /// Create TestRequest and set header
    pub fn with_header<K, V>(key: K, value: V) -> TestRequest<()>
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        TestRequest::default().header(key, value)
    }

    /// Create TestRequest and set request cookie
    pub fn with_cookie(cookie: Cookie<'static>) -> TestRequest<()> {
        TestRequest::default().cookie(cookie)
    }
}

impl<S: 'static> TestRequest<S> {
    /// Start HttpRequest build process with application state
    pub fn with_state(state: S) -> TestRequest<S> {
        TestRequest {
            state,
            method: Method::GET,
            uri: Uri::from_str("/").unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            params: Params::new(),
            cookies: None,
            payload: None,
            prefix: 0,
        }
    }

    /// Set HTTP version of this request
    pub fn version(mut self, ver: Version) -> Self {
        self.version = ver;
        self
    }

    /// Set HTTP method of this request
    pub fn method(mut self, meth: Method) -> Self {
        self.method = meth;
        self
    }

    /// Set HTTP Uri of this request
    pub fn uri(mut self, path: &str) -> Self {
        self.uri = Uri::from_str(path).unwrap();
        self
    }

    /// set cookie of this request
    pub fn cookie(mut self, cookie: Cookie<'static>) -> Self {
        if self.cookies.is_some() {
            let mut should_insert = true;
            let old_cookies = self.cookies.as_mut().unwrap();
            for old_cookie in old_cookies.iter() {
                if old_cookie == &cookie {
                    should_insert = false
                };
            };
            if should_insert {
                old_cookies.push(cookie);
            };
        } else {
            self.cookies = Some(vec![cookie]);
        };
        self
    }

    /// Set a header
    pub fn set<H: Header>(mut self, hdr: H) -> Self {
        if let Ok(value) = hdr.try_into() {
            self.headers.append(H::name(), value);
            return self;
        }
        panic!("Can not set header");
    }

    /// Set a header
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Ok(key) = HeaderName::try_from(key) {
            if let Ok(value) = value.try_into() {
                self.headers.append(key, value);
                return self;
            }
        }
        panic!("Can not create header");
    }

    /// Set request path pattern parameter
    pub fn param(mut self, name: &'static str, value: &'static str) -> Self {
        self.params.add_static(name, value);
        self
    }

    /// Set request payload
    pub fn set_payload<B: Into<Binary>>(mut self, data: B) -> Self {
        let mut data = data.into();
        let mut payload = Payload::empty();
        payload.unread_data(data.take());
        self.payload = Some(payload);
        self
    }

    /// Set request's prefix
    pub fn prefix(mut self, prefix: u16) -> Self {
        self.prefix = prefix;
        self
    }

    /// Complete request creation and generate `HttpRequest` instance
    pub fn finish(self) -> HttpRequest<S> {
        let TestRequest {
            state,
            method,
            uri,
            version,
            headers,
            mut params,
            cookies,
            payload,
            prefix,
        } = self;
        let router = Router::<()>::default();

        let pool = RequestPool::pool(ServerSettings::default());
        let mut req = RequestPool::get(pool);
        {
            let inner = req.inner_mut();
            inner.method = method;
            inner.url = InnerUrl::new(uri);
            inner.version = version;
            inner.headers = headers;
            *inner.payload.borrow_mut() = payload;
        }
        params.set_url(req.url().clone());
        let mut info = router.route_info_params(0, params);
        info.set_prefix(prefix);

        let mut req = HttpRequest::new(req, Rc::new(state), info);
        req.set_cookies(cookies);
        req
    }

    #[cfg(test)]
    /// Complete request creation and generate `HttpRequest` instance
    pub(crate) fn finish_with_router(self, router: Router<S>) -> HttpRequest<S> {
        let TestRequest {
            state,
            method,
            uri,
            version,
            headers,
            mut params,
            cookies,
            payload,
            prefix,
        } = self;

        let pool = RequestPool::pool(ServerSettings::default());
        let mut req = RequestPool::get(pool);
        {
            let inner = req.inner_mut();
            inner.method = method;
            inner.url = InnerUrl::new(uri);
            inner.version = version;
            inner.headers = headers;
            *inner.payload.borrow_mut() = payload;
        }
        params.set_url(req.url().clone());
        let mut info = router.route_info_params(0, params);
        info.set_prefix(prefix);
        let mut req = HttpRequest::new(req, Rc::new(state), info);
        req.set_cookies(cookies);
        req
    }

    /// Complete request creation and generate server `Request` instance
    pub fn request(self) -> Request {
        let TestRequest {
            method,
            uri,
            version,
            headers,
            payload,
            ..
        } = self;

        let pool = RequestPool::pool(ServerSettings::default());
        let mut req = RequestPool::get(pool);
        {
            let inner = req.inner_mut();
            inner.method = method;
            inner.url = InnerUrl::new(uri);
            inner.version = version;
            inner.headers = headers;
            *inner.payload.borrow_mut() = payload;
        }
        req
    }

    /// This method generates `HttpRequest` instance and runs handler
    /// with generated request.
    pub fn run<H: Handler<S>>(self, h: &H) -> Result<HttpResponse, Error> {
        let req = self.finish();
        let resp = h.handle(&req);

        match resp.respond_to(&req) {
            Ok(resp) => match resp.into().into() {
                AsyncResultItem::Ok(resp) => Ok(resp),
                AsyncResultItem::Err(err) => Err(err),
                AsyncResultItem::Future(fut) => {
                    let mut sys = System::new("test");
                    sys.block_on(fut)
                }
            },
            Err(err) => Err(err.into()),
        }
    }

    /// This method generates `HttpRequest` instance and runs handler
    /// with generated request.
    ///
    /// This method panics is handler returns actor.
    pub fn run_async<H, R, F, E>(self, h: H) -> Result<HttpResponse, E>
    where
        H: Fn(HttpRequest<S>) -> F + 'static,
        F: Future<Item = R, Error = E> + 'static,
        R: Responder<Error = E> + 'static,
        E: Into<Error> + 'static,
    {
        let req = self.finish();
        let fut = h(req.clone());

        let mut sys = System::new("test");
        match sys.block_on(fut) {
            Ok(r) => match r.respond_to(&req) {
                Ok(reply) => match reply.into().into() {
                    AsyncResultItem::Ok(resp) => Ok(resp),
                    _ => panic!("Nested async replies are not supported"),
                },
                Err(e) => Err(e),
            },
            Err(err) => Err(err),
        }
    }

    /// This method generates `HttpRequest` instance and executes handler
    pub fn run_async_result<F, R, I, E>(self, f: F) -> Result<I, E>
    where
        F: FnOnce(&HttpRequest<S>) -> R,
        R: Into<AsyncResult<I, E>>,
    {
        let req = self.finish();
        let res = f(&req);

        match res.into().into() {
            AsyncResultItem::Ok(resp) => Ok(resp),
            AsyncResultItem::Err(err) => Err(err),
            AsyncResultItem::Future(fut) => {
                let mut sys = System::new("test");
                sys.block_on(fut)
            }
        }
    }

    /// This method generates `HttpRequest` instance and executes handler
    pub fn execute<F, R>(self, f: F) -> Result<HttpResponse, Error>
    where
        F: FnOnce(&HttpRequest<S>) -> R,
        R: Responder + 'static,
    {
        let req = self.finish();
        let resp = f(&req);

        match resp.respond_to(&req) {
            Ok(resp) => match resp.into().into() {
                AsyncResultItem::Ok(resp) => Ok(resp),
                AsyncResultItem::Err(err) => Err(err),
                AsyncResultItem::Future(fut) => {
                    let mut sys = System::new("test");
                    sys.block_on(fut)
                }
            },
            Err(err) => Err(err.into()),
        }
    }
}
