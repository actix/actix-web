//! Various helpers for Actix applications to use during testing.
use std::cell::RefCell;
use std::sync::mpsc;
use std::{net, thread, time};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_rt::{Runtime, System};
use actix_server::{Server, StreamServiceFactory};
use awc::{error::PayloadError, ws, Client, ClientRequest, ClientResponse, Connector};
use bytes::Bytes;
use futures::future::lazy;
use futures::{Future, IntoFuture, Stream};
use http::Method;
use net2::TcpBuilder;
use tokio_tcp::TcpStream;

thread_local! {
    static RT: RefCell<Inner> = {
        RefCell::new(Inner(Some(Runtime::new().unwrap())))
    };
}

struct Inner(Option<Runtime>);

impl Inner {
    fn get_mut(&mut self) -> &mut Runtime {
        self.0.as_mut().unwrap()
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        std::mem::forget(self.0.take().unwrap())
    }
}

/// Runs the provided future, blocking the current thread until the future
/// completes.
///
/// This function can be used to synchronously block the current thread
/// until the provided `future` has resolved either successfully or with an
/// error. The result of the future is then returned from this function
/// call.
///
/// Note that this function is intended to be used only for testing purpose.
/// This function panics on nested call.
pub fn block_on<F>(f: F) -> Result<F::Item, F::Error>
where
    F: IntoFuture,
{
    RT.with(move |rt| rt.borrow_mut().get_mut().block_on(f.into_future()))
}

/// Runs the provided function, blocking the current thread until the resul
/// future completes.
///
/// This function can be used to synchronously block the current thread
/// until the provided `future` has resolved either successfully or with an
/// error. The result of the future is then returned from this function
/// call.
///
/// Note that this function is intended to be used only for testing purpose.
/// This function panics on nested call.
pub fn block_fn<F, R>(f: F) -> Result<R::Item, R::Error>
where
    F: FnOnce() -> R,
    R: IntoFuture,
{
    RT.with(move |rt| rt.borrow_mut().get_mut().block_on(lazy(f)))
}

/// The `TestServer` type.
///
/// `TestServer` is very simple test server that simplify process of writing
/// integration tests cases for actix web applications.
///
/// # Examples
///
/// ```rust
/// use actix_http::HttpService;
/// use actix_http_test::TestServer;
/// use actix_web::{web, App, HttpResponse};
///
/// fn my_handler() -> HttpResponse {
///     HttpResponse::Ok().into()
/// }
///
/// fn main() {
///     let mut srv = TestServer::new(
///         || HttpService::new(
///             App::new().service(
///                 web::resource("/").to(my_handler))
///         )
///     );
///
///     let req = srv.get("/");
///     let response = srv.block_on(req.send()).unwrap();
///     assert!(response.status().is_success());
/// }
/// ```
pub struct TestServer;

/// Test server controller
pub struct TestServerRuntime {
    addr: net::SocketAddr,
    rt: Runtime,
    client: Client,
}

impl TestServer {
    #[allow(clippy::new_ret_no_self)]
    /// Start new test server with application factory
    pub fn new<F: StreamServiceFactory<TcpStream>>(factory: F) -> TestServerRuntime {
        let (tx, rx) = mpsc::channel();

        // run server in separate thread
        thread::spawn(move || {
            let sys = System::new("actix-test-server");
            let tcp = net::TcpListener::bind("127.0.0.1:0").unwrap();
            let local_addr = tcp.local_addr().unwrap();

            Server::build()
                .listen("test", tcp, factory)?
                .workers(1)
                .disable_signals()
                .start();

            tx.send((System::current(), local_addr)).unwrap();
            sys.run()
        });

        let (system, addr) = rx.recv().unwrap();
        let mut rt = Runtime::new().unwrap();

        let client = rt
            .block_on(lazy(move || {
                let connector = {
                    #[cfg(feature = "ssl")]
                    {
                        use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

                        let mut builder =
                            SslConnector::builder(SslMethod::tls()).unwrap();
                        builder.set_verify(SslVerifyMode::NONE);
                        let _ = builder.set_alpn_protos(b"\x02h2\x08http/1.1").map_err(
                            |e| log::error!("Can not set alpn protocol: {:?}", e),
                        );
                        Connector::new()
                            .conn_lifetime(time::Duration::from_secs(0))
                            .timeout(time::Duration::from_millis(500))
                            .ssl(builder.build())
                            .finish()
                    }
                    #[cfg(not(feature = "ssl"))]
                    {
                        Connector::new()
                            .conn_lifetime(time::Duration::from_secs(0))
                            .timeout(time::Duration::from_millis(500))
                            .finish()
                    }
                };

                Ok::<Client, ()>(Client::build().connector(connector).finish())
            }))
            .unwrap();
        rt.block_on(lazy(
            || Ok::<_, ()>(actix_connect::start_default_resolver()),
        ))
        .unwrap();
        System::set_current(system);
        TestServerRuntime { addr, rt, client }
    }

    /// Get first available unused address
    pub fn unused_addr() -> net::SocketAddr {
        let addr: net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let socket = TcpBuilder::new_v4().unwrap();
        socket.bind(&addr).unwrap();
        socket.reuse_address(true).unwrap();
        let tcp = socket.to_tcp_listener().unwrap();
        tcp.local_addr().unwrap()
    }
}

impl TestServerRuntime {
    /// Execute future on current core
    pub fn block_on<F, I, E>(&mut self, fut: F) -> Result<I, E>
    where
        F: Future<Item = I, Error = E>,
    {
        self.rt.block_on(fut)
    }

    /// Execute future on current core
    pub fn block_on_fn<F, R>(&mut self, f: F) -> Result<R::Item, R::Error>
    where
        F: FnOnce() -> R,
        R: Future,
    {
        self.rt.block_on(lazy(f))
    }

    /// Execute function on current core
    pub fn execute<F, R>(&mut self, fut: F) -> R
    where
        F: FnOnce() -> R,
    {
        self.rt.block_on(lazy(|| Ok::<_, ()>(fut()))).unwrap()
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

    /// Construct test https server url
    pub fn surl(&self, uri: &str) -> String {
        if uri.starts_with('/') {
            format!("https://localhost:{}{}", self.addr.port(), uri)
        } else {
            format!("https://localhost:{}/{}", self.addr.port(), uri)
        }
    }

    /// Create `GET` request
    pub fn get<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.get(self.url(path.as_ref()).as_str())
    }

    /// Create https `GET` request
    pub fn sget<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.get(self.surl(path.as_ref()).as_str())
    }

    /// Create `POST` request
    pub fn post<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.post(self.url(path.as_ref()).as_str())
    }

    /// Create https `POST` request
    pub fn spost<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.post(self.surl(path.as_ref()).as_str())
    }

    /// Create `HEAD` request
    pub fn head<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.head(self.url(path.as_ref()).as_str())
    }

    /// Create https `HEAD` request
    pub fn shead<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.head(self.surl(path.as_ref()).as_str())
    }

    /// Create `PUT` request
    pub fn put<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.put(self.url(path.as_ref()).as_str())
    }

    /// Create https `PUT` request
    pub fn sput<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.put(self.surl(path.as_ref()).as_str())
    }

    /// Create `PATCH` request
    pub fn patch<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.patch(self.url(path.as_ref()).as_str())
    }

    /// Create https `PATCH` request
    pub fn spatch<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.patch(self.surl(path.as_ref()).as_str())
    }

    /// Create `DELETE` request
    pub fn delete<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.delete(self.url(path.as_ref()).as_str())
    }

    /// Create https `DELETE` request
    pub fn sdelete<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.delete(self.surl(path.as_ref()).as_str())
    }

    /// Create `OPTIONS` request
    pub fn options<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.options(self.url(path.as_ref()).as_str())
    }

    /// Create https `OPTIONS` request
    pub fn soptions<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.options(self.surl(path.as_ref()).as_str())
    }

    /// Connect to test http server
    pub fn request<S: AsRef<str>>(&self, method: Method, path: S) -> ClientRequest {
        self.client.request(method, path.as_ref())
    }

    pub fn load_body<S>(
        &mut self,
        mut response: ClientResponse<S>,
    ) -> Result<Bytes, PayloadError>
    where
        S: Stream<Item = Bytes, Error = PayloadError> + 'static,
    {
        self.block_on(response.body().limit(10_485_760))
    }

    /// Connect to websocket server at a given path
    pub fn ws_at(
        &mut self,
        path: &str,
    ) -> Result<Framed<impl AsyncRead + AsyncWrite, ws::Codec>, awc::error::WsClientError>
    {
        let url = self.url(path);
        let connect = self.client.ws(url).connect();
        self.rt
            .block_on(lazy(move || connect.map(|(_, framed)| framed)))
    }

    /// Connect to a websocket server
    pub fn ws(
        &mut self,
    ) -> Result<Framed<impl AsyncRead + AsyncWrite, ws::Codec>, awc::error::WsClientError>
    {
        self.ws_at("/")
    }

    /// Stop http server
    fn stop(&mut self) {
        System::current().stop();
    }
}

impl Drop for TestServerRuntime {
    fn drop(&mut self) {
        self.stop()
    }
}
