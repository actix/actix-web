//! Various helpers for Actix applications to use during testing.

use std::{net, thread};
use std::rc::Rc;
use std::sync::mpsc;
use std::str::FromStr;

use actix::{Arbiter, Addr, Syn, System, SystemRunner, msgs};
use cookie::Cookie;
use http::{Uri, Method, Version, HeaderMap, HttpTryFrom};
use http::header::{HeaderName, HeaderValue};
use futures::Future;
use tokio_core::net::TcpListener;
use tokio_core::reactor::Core;
use net2::TcpBuilder;

use body::Binary;
use error::Error;
use handler::{Handler, Responder, ReplyItem};
use middleware::Middleware;
use application::{Application, HttpApplication};
use param::Params;
use router::Router;
use payload::Payload;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use server::{HttpServer, IntoHttpHandler, ServerSettings};
use ws::{WsClient, WsClientError, WsClientReader, WsClientWriter};
use client::{ClientRequest, ClientRequestBuilder};

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
/// #     httpcodes::HTTPOk.into()
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
}

impl TestServer {

    /// Start new test server
    ///
    /// This method accepts configuration method. You can add
    /// middlewares or set handlers for test application.
    pub fn new<F>(config: F) -> Self
        where F: Sync + Send + 'static + Fn(&mut TestApp<()>),
    {
        TestServer::with_state(||(), config)
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
            let tcp = TcpListener::from_listener(tcp, &local_addr, Arbiter::handle()).unwrap();

            HttpServer::new(factory).disable_signals().start_incoming(tcp.incoming(), false);

            tx.send((Arbiter::system(), local_addr)).unwrap();
            let _ = sys.run();
        });

        let (sys, addr) = rx.recv().unwrap();
        TestServer {
            addr: addr,
            thread: Some(join),
            system: System::new("actix-test"),
            server_sys: sys,
        }
    }

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

        let (sys, addr) = rx.recv().unwrap();
        TestServer {
            addr: addr,
            thread: Some(join),
            system: System::new("actix-test"),
            server_sys: sys,
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
            format!("http://{}{}", self.addr, uri)
        } else {
            format!("http://{}/{}", self.addr, uri)
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
    pub fn ws(&mut self) -> Result<(WsClientReader, WsClientWriter), WsClientError> {
        let url = self.url("/");
        self.system.run_until_complete(WsClient::new(url).connect())
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
            .uri(self.url(path).as_str()).take()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop()
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
///         httpcodes::HTTPOk.into()
///     } else {
///         httpcodes::HTTPBadRequest.into()
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
    pub fn with_header<K, V>(key: K, value: V) -> TestRequest<()>
        where HeaderName: HttpTryFrom<K>,
              HeaderValue: HttpTryFrom<V>
    {
        TestRequest::default().header(key, value)
    }
}

impl<S> TestRequest<S> {

    /// Start HttpRequest build process with application state
    pub fn with_state(state: S) -> TestRequest<S> {
        TestRequest {
            state: state,
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
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
        where HeaderName: HttpTryFrom<K>,
              HeaderValue: HttpTryFrom<V>
    {
        if let Ok(key) = HeaderName::try_from(key) {
            if let Ok(value) = HeaderValue::try_from(value) {
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
    pub(crate) fn finish_no_router(self) -> HttpRequest<S> {
        let TestRequest { state, method, uri, version, headers, params, cookies, payload } = self;
        let req = HttpRequest::new(method, uri, version, headers, payload);
        req.as_mut().cookies = cookies;
        req.as_mut().params = params;
        req.with_state_no_router(Rc::new(state))
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

        match resp.respond_to(req.clone_without_state()) {
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
                match r.respond_to(req.clone_without_state()) {
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
