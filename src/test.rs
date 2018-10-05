//! Various helpers for Actix applications to use during testing.
use std::net;
use std::str::FromStr;

use actix::System;

use cookie::Cookie;
use futures::Future;
use http::header::HeaderName;
use http::{HeaderMap, HttpTryFrom, Method, Uri, Version};
use net2::TcpBuilder;
use tokio::runtime::current_thread::Runtime;

use body::Binary;
use header::{Header, IntoHeaderValue};
use payload::Payload;
use request::Request;
use uri::Url as InnerUrl;
// use ws;

/// The `TestServer` type.
///
/// `TestServer` is very simple test server that simplify process of writing
/// integration tests cases for actix web applications.
///
/// # Examples
///
/// ```rust,ignore
/// # extern crate actix_web;
/// # use actix_web::*;
/// #
/// # fn my_handler(req: &HttpRequest) -> Response {
/// #     Response::Ok().into()
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
    rt: Runtime,
    ssl: bool,
}

impl TestServer {
    /// Start new test server
    ///
    /// This method accepts configuration method. You can add
    /// middlewares or set handlers for test application.
    pub fn new<F>(_config: F) -> Self
    where
        F: Fn() + Clone + Send + 'static,
    {
        unimplemented!()
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
        System::current().stop();
    }

    /// Execute future on current core
    pub fn execute<F, I, E>(&mut self, fut: F) -> Result<I, E>
    where
        F: Future<Item = I, Error = E>,
    {
        self.rt.block_on(fut)
    }

    // /// Connect to websocket server at a given path
    // pub fn ws_at(
    //     &mut self, path: &str,
    // ) -> Result<(ws::ClientReader, ws::ClientWriter), ws::ClientError> {
    //     let url = self.url(path);
    //     self.rt
    //         .block_on(ws::Client::with_connector(url, self.conn.clone()).connect())
    // }

    // /// Connect to a websocket server
    // pub fn ws(
    //     &mut self,
    // ) -> Result<(ws::ClientReader, ws::ClientWriter), ws::ClientError> {
    //     self.ws_at("/")
    // }

    // /// Create `GET` request
    // pub fn get(&self) -> ClientRequestBuilder {
    //     ClientRequest::get(self.url("/").as_str())
    // }

    // /// Create `POST` request
    // pub fn post(&self) -> ClientRequestBuilder {
    //     ClientRequest::post(self.url("/").as_str())
    // }

    // /// Create `HEAD` request
    // pub fn head(&self) -> ClientRequestBuilder {
    //     ClientRequest::head(self.url("/").as_str())
    // }

    // /// Connect to test http server
    // pub fn client(&self, meth: Method, path: &str) -> ClientRequestBuilder {
    //     ClientRequest::build()
    //         .method(meth)
    //         .uri(self.url(path).as_str())
    //         .with_connector(self.conn.clone())
    //         .take()
    // }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop()
    }
}

// /// An `TestServer` builder
// ///
// /// This type can be used to construct an instance of `TestServer` through a
// /// builder-like pattern.
// pub struct TestServerBuilder<S, F>
// where
//     F: Fn() -> S + Send + Clone + 'static,
// {
//     state: F,
// }

// impl<S: 'static, F> TestServerBuilder<S, F>
// where
//     F: Fn() -> S + Send + Clone + 'static,
// {
//     /// Create a new test server
//     pub fn new(state: F) -> TestServerBuilder<S, F> {
//         TestServerBuilder { state }
//     }

//     #[allow(unused_mut)]
//     /// Configure test application and run test server
//     pub fn start<C>(mut self, config: C) -> TestServer
//     where
//         C: Fn(&mut TestApp<S>) + Clone + Send + 'static,
//     {
//         let (tx, rx) = mpsc::channel();

//         let mut has_ssl = false;

//         #[cfg(any(feature = "alpn", feature = "ssl"))]
//         {
//             has_ssl = has_ssl || self.ssl.is_some();
//         }

//         #[cfg(feature = "rust-tls")]
//         {
//             has_ssl = has_ssl || self.rust_ssl.is_some();
//         }

//         // run server in separate thread
//         thread::spawn(move || {
//             let addr = TestServer::unused_addr();

//             let sys = System::new("actix-test-server");
//             let state = self.state;
//             let mut srv = HttpServer::new(move || {
//                 let mut app = TestApp::new(state());
//                 config(&mut app);
//                 app
//             }).workers(1)
//             .keep_alive(5)
//             .disable_signals();

//             tx.send((System::current(), addr, TestServer::get_conn()))
//                 .unwrap();

//             #[cfg(any(feature = "alpn", feature = "ssl"))]
//             {
//                 let ssl = self.ssl.take();
//                 if let Some(ssl) = ssl {
//                     let tcp = net::TcpListener::bind(addr).unwrap();
//                     srv = srv.listen_ssl(tcp, ssl).unwrap();
//                 }
//             }
//             #[cfg(feature = "rust-tls")]
//             {
//                 let ssl = self.rust_ssl.take();
//                 if let Some(ssl) = ssl {
//                     let tcp = net::TcpListener::bind(addr).unwrap();
//                     srv = srv.listen_rustls(tcp, ssl);
//                 }
//             }
//             if !has_ssl {
//                 let tcp = net::TcpListener::bind(addr).unwrap();
//                 srv = srv.listen(tcp);
//             }
//             srv.start();

//             sys.run();
//         });

//         let (system, addr, conn) = rx.recv().unwrap();
//         System::set_current(system);
//         TestServer {
//             addr,
//             conn,
//             ssl: has_ssl,
//             rt: Runtime::new().unwrap(),
//         }
//     }
// }

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
            inner.method = method;
            inner.url = InnerUrl::new(uri);
            inner.version = version;
            inner.headers = headers;
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
