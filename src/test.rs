//! Various helpers for Actix applications to use during testing.

use std::{net, thread};
use std::rc::Rc;
use std::sync::mpsc;
use std::str::FromStr;

use actix::{Actor, Arbiter, Addr, Syn, System, SystemRunner, Unsync, msgs};
use cookie::Cookie;
use http::{Uri, Method, Version, HeaderMap, HttpTryFrom};
use http::header::HeaderName;
use futures::Future;
use tokio_core::net::TcpListener;
use tokio_core::reactor::Core;
use net2::TcpBuilder;

#[cfg(feature="alpn")]
use openssl::ssl::SslAcceptor;

use ws;
use body::Binary;
use error::Error;
use header::{Header, IntoHeaderValue};
use handler::{Handler, Responder, ReplyItem};
use middleware::Middleware;
use application::{Application, HttpApplication};
use param::Params;
use router::Router;
use payload::Payload;
use resource::Resource;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use server::{HttpServer, IntoHttpHandler, ServerSettings};
use client::{ClientRequest, ClientRequestBuilder, ClientConnector};

/// The `TestServer` type.
///
/// `TestServer` is very simple test server that simplify process of writing
/// integration tests cases for actix web applications.
///
/// # Examples
///
/// ```rust
/// # extern crate actix;
/// # extern crate actix_web;
/// # use actix_web::*;
/// #
/// # fn my_handler(req: HttpRequest) -> HttpResponse {
/// #     httpcodes::HttpOk.into()
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
    thread: Option<thread::JoinHandle<()>>,
    system: SystemRunner,
    server_sys: Addr<Syn, System>,
    ssl: bool,
    conn: Addr<Unsync, ClientConnector>,
}

impl TestServer {

    /// Start new test server
    ///
    /// This method accepts configuration method. You can add
    /// middlewares or set handlers for test application.
    pub fn new<F>(config: F) -> Self
        where F: Sync + Send + 'static + Fn(&mut TestApp<()>)
    {
        TestServerBuilder::new(||()).start(config)
    }

    /// Create test server builder
    pub fn build() -> TestServerBuilder<()> {
        TestServerBuilder::new(||())
    }

    /// Create test server builder with specific state factory
    ///
    /// This method can be used for constructing application state.
    /// Also it can be used for external dependecy initialization,
    /// like creating sync actors for diesel integration.
    pub fn build_with_state<F, S>(state: F) -> TestServerBuilder<S>
        where F: Fn() -> S + Sync + Send + 'static,
              S: 'static,
    {
        TestServerBuilder::new(state)
    }

    /// Start new test server with application factory
    pub fn with_factory<F, U, H>(factory: F) -> Self
        where F: Fn() -> U + Sync + Send + 'static,
              U: IntoIterator<Item=H> + 'static,
              H: IntoHttpHandler + 'static,
    {
        let (tx, rx) = mpsc::channel();

        // run server in separate thread
        let join = thread::spawn(move || {
            let sys = System::new("actix-test-server");
            let tcp = net::TcpListener::bind("127.0.0.1:0").unwrap();
            let local_addr = tcp.local_addr().unwrap();
            let tcp = TcpListener::from_listener(
                tcp, &local_addr, Arbiter::handle()).unwrap();

            HttpServer::new(factory)
                .disable_signals()
                .start_incoming(tcp.incoming(), false);

            tx.send((Arbiter::system(), local_addr)).unwrap();
            let _ = sys.run();
        });

        let sys = System::new("actix-test");
        let (server_sys, addr) = rx.recv().unwrap();
        TestServer {
            addr,
            server_sys,
            ssl: false,
            conn: TestServer::get_conn(),
            thread: Some(join),
            system: sys,
        }
    }

    #[deprecated(since="0.4.10",
                 note="please use `TestServer::build_with_state()` instead")]
    /// Start new test server with custom application state
    ///
    /// This method accepts state factory and configuration method.
    pub fn with_state<S, FS, F>(state: FS, config: F) -> Self
        where S: 'static,
              FS: Sync + Send + 'static + Fn() -> S,
              F: Sync + Send + 'static + Fn(&mut TestApp<S>),
    {
        let (tx, rx) = mpsc::channel();

        // run server in separate thread
        let join = thread::spawn(move || {
            let sys = System::new("actix-test-server");

            let tcp = net::TcpListener::bind("127.0.0.1:0").unwrap();
            let local_addr = tcp.local_addr().unwrap();
            let tcp = TcpListener::from_listener(tcp, &local_addr, Arbiter::handle()).unwrap();

            HttpServer::new(move || {
                let mut app = TestApp::new(state());
                config(&mut app);
                vec![app]}
            ).disable_signals().start_incoming(tcp.incoming(), false);

            tx.send((Arbiter::system(), local_addr)).unwrap();
            let _ = sys.run();
        });

        let system = System::new("actix-test");
        let (server_sys, addr) = rx.recv().unwrap();
        TestServer {
            addr,
            server_sys,
            system,
            ssl: false,
            conn: TestServer::get_conn(),
            thread: Some(join),
        }
    }

    fn get_conn() -> Addr<Unsync, ClientConnector> {
        #[cfg(feature="alpn")]
        {
            use openssl::ssl::{SslMethod, SslConnector, SslVerifyMode};

            let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
            builder.set_verify(SslVerifyMode::NONE);
            ClientConnector::with_connector(builder.build()).start()
        }
        #[cfg(not(feature="alpn"))]
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
            format!("{}://{}{}", if self.ssl {"https"} else {"http"}, self.addr, uri)
        } else {
            format!("{}://{}/{}", if self.ssl {"https"} else {"http"}, self.addr, uri)
        }
    }

    /// Stop http server
    fn stop(&mut self) {
        if let Some(handle) = self.thread.take() {
            self.server_sys.do_send(msgs::SystemExit(0));
            let _ = handle.join();
        }
    }

    /// Execute future on current core
    pub fn execute<F, I, E>(&mut self, fut: F) -> Result<I, E>
        where F: Future<Item=I, Error=E>
    {
        self.system.run_until_complete(fut)
    }

    /// Connect to websocket server
    pub fn ws(&mut self) -> Result<(ws::ClientReader, ws::ClientWriter), ws::ClientError> {
        let url = self.url("/");
        self.system.run_until_complete(
            ws::Client::with_connector(url, self.conn.clone()).connect())
    }

    /// Create `GET` request
    pub fn get(&self) -> ClientRequestBuilder {
        ClientRequest::get(self.url("/").as_str())
    }

    /// Create `POST` request
    pub fn post(&self) -> ClientRequestBuilder {
        ClientRequest::get(self.url("/").as_str())
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
pub struct TestServerBuilder<S> {
    state: Box<Fn() -> S + Sync + Send + 'static>,
    #[cfg(feature="alpn")]
    ssl: Option<SslAcceptor>,
}

impl<S: 'static> TestServerBuilder<S> {

    pub fn new<F>(state: F) -> TestServerBuilder<S>
        where F: Fn() -> S + Sync + Send + 'static
    {
        TestServerBuilder {
            state: Box::new(state),
            #[cfg(feature="alpn")]
            ssl: None,
        }
    }

    #[cfg(feature="alpn")]
    /// Create ssl server
    pub fn ssl(mut self, ssl: SslAcceptor) -> Self {
        self.ssl = Some(ssl);
        self
    }

    #[allow(unused_mut)]
    /// Configure test application and run test server
    pub fn start<F>(mut self, config: F) -> TestServer
        where F: Sync + Send + 'static + Fn(&mut TestApp<S>),
    {
        let (tx, rx) = mpsc::channel();

        #[cfg(feature="alpn")]
        let ssl = self.ssl.is_some();
        #[cfg(not(feature="alpn"))]
        let ssl = false;

        // run server in separate thread
        let join = thread::spawn(move || {
            let sys = System::new("actix-test-server");

            let tcp = net::TcpListener::bind("127.0.0.1:0").unwrap();
            let local_addr = tcp.local_addr().unwrap();
            let tcp = TcpListener::from_listener(
                tcp, &local_addr, Arbiter::handle()).unwrap();

            let state = self.state;

            let srv = HttpServer::new(move || {
                let mut app = TestApp::new(state());
                config(&mut app);
                vec![app]})
                .disable_signals();

            #[cfg(feature="alpn")]
            {
                use std::io;
                use futures::Stream;
                use tokio_openssl::SslAcceptorExt;

                let ssl = self.ssl.take();
                if let Some(ssl) = ssl {
                    srv.start_incoming(
                        tcp.incoming()
                            .and_then(move |(sock, addr)| {
                                ssl.accept_async(sock)
                                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
                                    .map(move |s| (s, addr))
                            }),
                        false);
                } else {
                    srv.start_incoming(tcp.incoming(), false);
                }
            }
            #[cfg(not(feature="alpn"))]
            {
                srv.start_incoming(tcp.incoming(), false);
            }

            tx.send((Arbiter::system(), local_addr)).unwrap();
            let _ = sys.run();
        });

        let system = System::new("actix-test");
        let (server_sys, addr) = rx.recv().unwrap();
        TestServer {
            addr,
            server_sys,
            ssl,
            system,
            conn: TestServer::get_conn(),
            thread: Some(join),
        }
    }
}

/// Test application helper for testing request handlers.
pub struct TestApp<S=()> {
    app: Option<Application<S>>,
}

impl<S: 'static> TestApp<S> {
    fn new(state: S) -> TestApp<S> {
        let app = Application::with_state(state);
        TestApp{app: Some(app)}
    }

    /// Register handler for "/"
    pub fn handler<H: Handler<S>>(&mut self, handler: H) {
        self.app = Some(self.app.take().unwrap().resource("/", |r| r.h(handler)));
    }

    /// Register handler for "/" with resource middleware
    pub fn handler2<H, M>(&mut self, handler: H, mw: M)
        where H: Handler<S>, M: Middleware<S>
    {
        self.app = Some(self.app.take().unwrap()
                        .resource("/", |r| {
                            r.middleware(mw);
                            r.h(handler)}));
    }

    /// Register middleware
    pub fn middleware<T>(&mut self, mw: T) -> &mut TestApp<S>
        where T: Middleware<S> + 'static
    {
        self.app = Some(self.app.take().unwrap().middleware(mw));
        self
    }

    /// Register resource. This method is similar
    /// to `Application::resource()` method.
    pub fn resource<F>(&mut self, path: &str, f: F) -> &mut TestApp<S>
        where F: FnOnce(&mut Resource<S>) + 'static
    {
        self.app = Some(self.app.take().unwrap().resource(path, f));
        self
    }
}

impl<S: 'static> IntoHttpHandler for TestApp<S> {
    type Handler = HttpApplication<S>;

    fn into_handler(mut self, settings: ServerSettings) -> HttpApplication<S> {
        self.app.take().unwrap().into_handler(settings)
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
/// fn index(req: HttpRequest) -> HttpResponse {
///     if let Some(hdr) = req.headers().get(header::CONTENT_TYPE) {
///         httpcodes::HttpOk.into()
///     } else {
///         httpcodes::HttpBadRequest.into()
///     }
/// }
///
/// fn main() {
///     let resp = TestRequest::with_header("content-type", "text/plain")
///         .run(index).unwrap();
///     assert_eq!(resp.status(), StatusCode::OK);
///
///     let resp = TestRequest::default()
///         .run(index).unwrap();
///     assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
/// }
/// ```
pub struct TestRequest<S> {
    state: S,
    version: Version,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    params: Params<'static>,
    cookies: Option<Vec<Cookie<'static>>>,
    payload: Option<Payload>,
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
        }
    }
}

impl TestRequest<()> {

    /// Create TestRequest and set request uri
    pub fn with_uri(path: &str) -> TestRequest<()> {
        TestRequest::default().uri(path)
    }

    /// Create TestRequest and set header
    pub fn with_hdr<H: Header>(hdr: H) -> TestRequest<()>
    {
        TestRequest::default().set(hdr)
    }

    /// Create TestRequest and set header
    pub fn with_header<K, V>(key: K, value: V) -> TestRequest<()>
        where HeaderName: HttpTryFrom<K>, V: IntoHeaderValue,
    {
        TestRequest::default().header(key, value)
    }
}

impl<S> TestRequest<S> {

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

    /// Set a header
    pub fn set<H: Header>(mut self, hdr: H) -> Self
    {
        if let Ok(value) = hdr.try_into() {
            self.headers.append(H::name(), value);
            return self
        }
        panic!("Can not set header");
    }

    /// Set a header
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
        where HeaderName: HttpTryFrom<K>, V: IntoHeaderValue
    {
        if let Ok(key) = HeaderName::try_from(key) {
            if let Ok(value) = value.try_into() {
                self.headers.append(key, value);
                return self
            }
        }
        panic!("Can not create header");
    }

    /// Set request path pattern parameter
    pub fn param(mut self, name: &'static str, value: &'static str) -> Self {
        self.params.add(name, value);
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

    /// Complete request creation and generate `HttpRequest` instance
    pub fn finish(self) -> HttpRequest<S> {
        let TestRequest { state, method, uri, version, headers, params, cookies, payload } = self;
        let req = HttpRequest::new(method, uri, version, headers, payload);
        req.as_mut().cookies = cookies;
        req.as_mut().params = params;
        let (router, _) = Router::new::<S>("/", ServerSettings::default(), Vec::new());
        req.with_state(Rc::new(state), router)
    }

    #[cfg(test)]
    /// Complete request creation and generate `HttpRequest` instance
    pub(crate) fn finish_with_router(self, router: Router) -> HttpRequest<S> {
        let TestRequest { state, method, uri,
                          version, headers, params, cookies, payload } = self;

        let req = HttpRequest::new(method, uri, version, headers, payload);
        req.as_mut().cookies = cookies;
        req.as_mut().params = params;
        req.with_state(Rc::new(state), router)
    }

    /// This method generates `HttpRequest` instance and runs handler
    /// with generated request.
    ///
    /// This method panics is handler returns actor or async result.
    pub fn run<H: Handler<S>>(self, mut h: H) ->
        Result<HttpResponse, <<H as Handler<S>>::Result as Responder>::Error>
    {
        let req = self.finish();
        let resp = h.handle(req.clone());

        match resp.respond_to(req.without_state()) {
            Ok(resp) => {
                match resp.into().into() {
                    ReplyItem::Message(resp) => Ok(resp),
                    ReplyItem::Future(_) => panic!("Async handler is not supported."),
                }
            },
            Err(err) => Err(err),
        }
    }

    /// This method generates `HttpRequest` instance and runs handler
    /// with generated request.
    ///
    /// This method panics is handler returns actor.
    pub fn run_async<H, R, F, E>(self, h: H) -> Result<HttpResponse, E>
        where H: Fn(HttpRequest<S>) -> F + 'static,
              F: Future<Item=R, Error=E> + 'static,
              R: Responder<Error=E> + 'static,
              E: Into<Error> + 'static
    {
        let req = self.finish();
        let fut = h(req.clone());

        let mut core = Core::new().unwrap();
        match core.run(fut) {
            Ok(r) => {
                match r.respond_to(req.without_state()) {
                    Ok(reply) => match reply.into().into() {
                        ReplyItem::Message(resp) => Ok(resp),
                        _ => panic!("Nested async replies are not supported"),
                    },
                    Err(e) => Err(e),
                }
            },
            Err(err) => Err(err),
        }
    }
}
