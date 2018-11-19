//! Various helpers for Actix applications to use during testing.
use std::str::FromStr;
use std::sync::mpsc;
use std::{net, thread};

use actix::System;
use actix_net::codec::Framed;
use actix_net::server::{Server, StreamServiceFactory};
use actix_net::service::Service;

use bytes::Bytes;
use cookie::Cookie;
use futures::future::{lazy, Future};
use http::header::HeaderName;
use http::{HeaderMap, HttpTryFrom, Method, Uri, Version};
use net2::TcpBuilder;
use tokio::runtime::current_thread::Runtime;
use tokio_io::{AsyncRead, AsyncWrite};

use body::MessageBody;
use client::{
    ClientRequest, ClientRequestBuilder, ClientResponse, Connect, Connection, Connector,
    ConnectorError, SendRequestError,
};
use header::{Header, IntoHeaderValue};
use payload::Payload;
use request::Request;
use ws;

/// Test `Request` builder
///
/// ```rust,ignore
/// # extern crate http;
/// # extern crate actix_web;
/// # use http::{header, StatusCode};
/// # use actix_web::*;
/// use actix_web::test::TestRequest;
///
/// fn index(req: &HttpRequest) -> Response {
///     if let Some(hdr) = req.headers().get(header::CONTENT_TYPE) {
///         Response::Ok().into()
///     } else {
///         Response::BadRequest().into()
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
pub struct TestRequest {
    version: Version,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    _cookies: Option<Vec<Cookie<'static>>>,
    payload: Option<Payload>,
    prefix: u16,
}

impl Default for TestRequest {
    fn default() -> TestRequest {
        TestRequest {
            method: Method::GET,
            uri: Uri::from_str("/").unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            _cookies: None,
            payload: None,
            prefix: 0,
        }
    }
}

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

    /// Set request payload
    pub fn set_payload<B: Into<Bytes>>(mut self, data: B) -> Self {
        let mut payload = Payload::empty();
        payload.unread_data(data.into());
        self.payload = Some(payload);
        self
    }

    /// Set request's prefix
    pub fn prefix(mut self, prefix: u16) -> Self {
        self.prefix = prefix;
        self
    }

    /// Complete request creation and generate `Request` instance
    pub fn finish(self) -> Request {
        let TestRequest {
            method,
            uri,
            version,
            headers,
            _cookies: _,
            payload,
            prefix: _,
        } = self;

        let mut req = Request::new();
        {
            let inner = req.inner_mut();
            inner.head.uri = uri;
            inner.head.method = method;
            inner.head.version = version;
            inner.head.headers = headers;
            *inner.payload.borrow_mut() = payload;
        }
        // req.set_cookies(cookies);
        req
    }

    // /// This method generates `HttpRequest` instance and runs handler
    // /// with generated request.
    // pub fn run<H: Handler<S>>(self, h: &H) -> Result<Response, Error> {
    //     let req = self.finish();
    //     let resp = h.handle(&req);

    //     match resp.respond_to(&req) {
    //         Ok(resp) => match resp.into().into() {
    //             AsyncResultItem::Ok(resp) => Ok(resp),
    //             AsyncResultItem::Err(err) => Err(err),
    //             AsyncResultItem::Future(fut) => {
    //                 let mut sys = System::new("test");
    //                 sys.block_on(fut)
    //             }
    //         },
    //         Err(err) => Err(err.into()),
    //     }
    // }

    // /// This method generates `HttpRequest` instance and runs handler
    // /// with generated request.
    // ///
    // /// This method panics is handler returns actor.
    // pub fn run_async<H, R, F, E>(self, h: H) -> Result<Response, E>
    // where
    //     H: Fn(HttpRequest<S>) -> F + 'static,
    //     F: Future<Item = R, Error = E> + 'static,
    //     R: Responder<Error = E> + 'static,
    //     E: Into<Error> + 'static,
    // {
    //     let req = self.finish();
    //     let fut = h(req.clone());

    //     let mut sys = System::new("test");
    //     match sys.block_on(fut) {
    //         Ok(r) => match r.respond_to(&req) {
    //             Ok(reply) => match reply.into().into() {
    //                 AsyncResultItem::Ok(resp) => Ok(resp),
    //                 _ => panic!("Nested async replies are not supported"),
    //             },
    //             Err(e) => Err(e),
    //         },
    //         Err(err) => Err(err),
    //     }
    // }

    // /// This method generates `HttpRequest` instance and executes handler
    // pub fn run_async_result<F, R, I, E>(self, f: F) -> Result<I, E>
    // where
    //     F: FnOnce(&HttpRequest<S>) -> R,
    //     R: Into<AsyncResult<I, E>>,
    // {
    //     let req = self.finish();
    //     let res = f(&req);

    //     match res.into().into() {
    //         AsyncResultItem::Ok(resp) => Ok(resp),
    //         AsyncResultItem::Err(err) => Err(err),
    //         AsyncResultItem::Future(fut) => {
    //             let mut sys = System::new("test");
    //             sys.block_on(fut)
    //         }
    //     }
    // }

    // /// This method generates `HttpRequest` instance and executes handler
    // pub fn execute<F, R>(self, f: F) -> Result<Response, Error>
    // where
    //     F: FnOnce(&HttpRequest<S>) -> R,
    //     R: Responder + 'static,
    // {
    //     let req = self.finish();
    //     let resp = f(&req);

    //     match resp.respond_to(&req) {
    //         Ok(resp) => match resp.into().into() {
    //             AsyncResultItem::Ok(resp) => Ok(resp),
    //             AsyncResultItem::Err(err) => Err(err),
    //             AsyncResultItem::Future(fut) => {
    //                 let mut sys = System::new("test");
    //                 sys.block_on(fut)
    //             }
    //         },
    //         Err(err) => Err(err.into()),
    //     }
    // }
}

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
pub struct TestServer;

///
pub struct TestServerRuntime<T> {
    addr: net::SocketAddr,
    conn: T,
    rt: Runtime,
}

impl TestServer {
    /// Start new test server with application factory
    pub fn with_factory<F: StreamServiceFactory>(
        factory: F,
    ) -> TestServerRuntime<
        impl Service<Request = Connect, Response = impl Connection, Error = ConnectorError>
            + Clone,
    > {
        let (tx, rx) = mpsc::channel();

        // run server in separate thread
        thread::spawn(move || {
            let sys = System::new("actix-test-server");
            let tcp = net::TcpListener::bind("127.0.0.1:0").unwrap();
            let local_addr = tcp.local_addr().unwrap();

            Server::default()
                .listen("test", tcp, factory)
                .workers(1)
                .disable_signals()
                .start();

            tx.send((System::current(), local_addr)).unwrap();
            sys.run();
        });

        let (system, addr) = rx.recv().unwrap();
        System::set_current(system);

        let mut rt = Runtime::new().unwrap();
        let conn = rt
            .block_on(lazy(|| Ok::<_, ()>(TestServer::new_connector())))
            .unwrap();

        TestServerRuntime { addr, conn, rt }
    }

    fn new_connector(
) -> impl Service<Request = Connect, Response = impl Connection, Error = ConnectorError>
             + Clone {
        #[cfg(feature = "ssl")]
        {
            use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

            let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
            builder.set_verify(SslVerifyMode::NONE);
            Connector::default().ssl(builder.build()).service()
        }
        #[cfg(not(feature = "ssl"))]
        {
            Connector::default().service()
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
}

impl<T> TestServerRuntime<T> {
    /// Execute future on current core
    pub fn block_on<F, I, E>(&mut self, fut: F) -> Result<I, E>
    where
        F: Future<Item = I, Error = E>,
    {
        self.rt.block_on(fut)
    }

    /// Construct test server url
    pub fn addr(&self) -> net::SocketAddr {
        self.addr
    }

    /// Construct test server url
    pub fn url(&self, uri: &str) -> String {
        if uri.starts_with('/') {
            format!("http://localhost:{}{}", self.addr.port(), uri)
        } else {
            format!("http://localhost:{}/{}", self.addr.port(), uri)
        }
    }

    /// Create `GET` request
    pub fn get(&self) -> ClientRequestBuilder {
        ClientRequest::get(self.url("/").as_str())
    }

    /// Create `POST` request
    pub fn post(&self) -> ClientRequestBuilder {
        ClientRequest::post(self.url("/").as_str())
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
            .take()
    }

    /// Http connector
    pub fn connector(&mut self) -> &mut T {
        &mut self.conn
    }

    /// Http connector
    pub fn new_connector(&mut self) -> T
    where
        T: Clone,
    {
        self.conn.clone()
    }

    /// Stop http server
    fn stop(&mut self) {
        System::current().stop();
    }
}

impl<T> TestServerRuntime<T>
where
    T: Service<Request = Connect, Error = ConnectorError> + Clone,
    T::Response: Connection,
{
    /// Connect to websocket server at a given path
    pub fn ws_at(
        &mut self,
        path: &str,
    ) -> Result<Framed<impl AsyncRead + AsyncWrite, ws::Codec>, ws::ClientError> {
        let url = self.url(path);
        self.rt
            .block_on(lazy(|| ws::Client::default().call(ws::Connect::new(url))))
    }

    /// Connect to a websocket server
    pub fn ws(
        &mut self,
    ) -> Result<Framed<impl AsyncRead + AsyncWrite, ws::Codec>, ws::ClientError> {
        self.ws_at("/")
    }

    /// Send request and read response message
    pub fn send_request<B: MessageBody>(
        &mut self,
        req: ClientRequest<B>,
    ) -> Result<ClientResponse, SendRequestError> {
        self.rt.block_on(req.send(&mut self.conn))
    }
}

impl<T> Drop for TestServerRuntime<T> {
    fn drop(&mut self) {
        self.stop()
    }
}
