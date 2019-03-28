//! Various helpers for Actix applications to use during testing.
use std::sync::mpsc;
use std::{net, thread, time};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::client::Connector;
use actix_http::ws;
use actix_rt::{Runtime, System};
use actix_server::{Server, StreamServiceFactory};
use awc::{Client, ClientRequest};
use futures::future::{lazy, Future};
use http::Method;
use net2::TcpBuilder;

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
/// #
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
///     let req = srv.get();
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
    /// Start new test server with application factory
    pub fn new<F: StreamServiceFactory>(factory: F) -> TestServerRuntime {
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
                            .timeout(time::Duration::from_millis(500))
                            .ssl(builder.build())
                            .service()
                    }
                    #[cfg(not(feature = "ssl"))]
                    {
                        Connector::new()
                            .timeout(time::Duration::from_millis(500))
                            .service()
                    }
                };

                Ok::<Client, ()>(Client::build().connector(connector).finish())
            }))
            .unwrap();
        System::set_current(system);
        TestServerRuntime { addr, rt, client }
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

impl TestServerRuntime {
    /// Execute future on current core
    pub fn block_on<F, I, E>(&mut self, fut: F) -> Result<I, E>
    where
        F: Future<Item = I, Error = E>,
    {
        self.rt.block_on(fut)
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
            format!("http://127.0.0.1:{}{}", self.addr.port(), uri)
        } else {
            format!("http://127.0.0.1:{}/{}", self.addr.port(), uri)
        }
    }

    /// Construct test https server url
    pub fn surl(&self, uri: &str) -> String {
        if uri.starts_with('/') {
            format!("https://127.0.0.1:{}{}", self.addr.port(), uri)
        } else {
            format!("https://127.0.0.1:{}/{}", self.addr.port(), uri)
        }
    }

    /// Create `GET` request
    pub fn get(&self) -> ClientRequest {
        self.client.get(self.url("/").as_str())
    }

    /// Create https `GET` request
    pub fn sget(&self) -> ClientRequest {
        self.client.get(self.surl("/").as_str())
    }

    /// Create `POST` request
    pub fn post(&self) -> ClientRequest {
        self.client.post(self.url("/").as_str())
    }

    /// Create https `POST` request
    pub fn spost(&self) -> ClientRequest {
        self.client.post(self.surl("/").as_str())
    }

    /// Create `HEAD` request
    pub fn head(&self) -> ClientRequest {
        self.client.head(self.url("/").as_str())
    }

    /// Create https `HEAD` request
    pub fn shead(&self) -> ClientRequest {
        self.client.head(self.surl("/").as_str())
    }

    /// Connect to test http server
    pub fn request<S: AsRef<str>>(&self, method: Method, path: S) -> ClientRequest {
        self.client.request(method, path.as_ref())
    }

    /// Stop http server
    fn stop(&mut self) {
        System::current().stop();
    }
}

impl TestServerRuntime {
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
}

impl Drop for TestServerRuntime {
    fn drop(&mut self) {
        self.stop()
    }
}
