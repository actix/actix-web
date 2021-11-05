//! Integration testing tools for Actix Web applications.
//!
//! The main integration testing tool is [`TestServer`]. It spawns a real HTTP server on an
//! unused port and provides methods that use a real HTTP client. Therefore, it is much closer to
//! real-world cases than using `init_service`, which skips HTTP encoding and decoding.
//!
//! # Examples
//! ```
//! use actix_web::{get, web, test, App, HttpResponse, Error, Responder};
//!
//! #[get("/")]
//! async fn my_handler() -> Result<impl Responder, Error> {
//!     Ok(HttpResponse::Ok())
//! }
//!
//! #[actix_rt::test]
//! async fn test_example() {
//!     let srv = actix_test::start(||
//!         App::new().service(my_handler)
//!     );
//!
//!     let req = srv.get("/");
//!     let res = req.send().await.unwrap();
//!
//!     assert!(res.status().is_success());
//! }
//! ```

#[cfg(feature = "openssl")]
extern crate tls_openssl as openssl;
#[cfg(feature = "rustls")]
extern crate tls_rustls as rustls;

use std::{error::Error as StdError, fmt, net, sync::mpsc, thread, time};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
pub use actix_http::test::TestBuffer;
use actix_http::{
    http::{HeaderMap, Method},
    ws, HttpService, Request, Response,
};
use actix_service::{map_config, IntoServiceFactory, ServiceFactory, ServiceFactoryExt as _};
use actix_web::{
    dev::{AppConfig, MessageBody, Server, Service},
    rt, web, Error,
};
use awc::{error::PayloadError, Client, ClientRequest, ClientResponse, Connector};
use futures_core::Stream;

pub use actix_http_test::unused_addr;
pub use actix_web::test::{
    call_service, default_service, init_service, load_stream, ok_service, read_body,
    read_body_json, read_response, read_response_json, TestRequest,
};

/// Start default [`TestServer`].
///
/// # Examples
/// ```
/// use actix_web::{get, web, test, App, HttpResponse, Error, Responder};
///
/// #[get("/")]
/// async fn my_handler() -> Result<impl Responder, Error> {
///     Ok(HttpResponse::Ok())
/// }
///
/// #[actix_web::test]
/// async fn test_example() {
///     let srv = actix_test::start(||
///         App::new().service(my_handler)
///     );
///
///     let req = srv.get("/");
///     let res = req.send().await.unwrap();
///
///     assert!(res.status().is_success());
/// }
/// ```
pub fn start<F, I, S, B>(factory: F) -> TestServer
where
    F: Fn() -> I + Send + Clone + 'static,
    I: IntoServiceFactory<S, Request>,
    S: ServiceFactory<Request, Config = AppConfig> + 'static,
    S::Error: Into<Error> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,
    B: MessageBody + 'static,
    B::Error: Into<Box<dyn StdError>>,
{
    start_with(TestServerConfig::default(), factory)
}

/// Start test server with custom configuration
///
/// Check [`TestServerConfig`] docs for configuration options.
///
/// # Examples
/// ```
/// use actix_web::{get, web, test, App, HttpResponse, Error, Responder};
///
/// #[get("/")]
/// async fn my_handler() -> Result<impl Responder, Error> {
///     Ok(HttpResponse::Ok())
/// }
///
/// #[actix_web::test]
/// async fn test_example() {
///     let srv = actix_test::start_with(actix_test::config().h1(), ||
///         App::new().service(my_handler)
///     );
///
///     let req = srv.get("/");
///     let res = req.send().await.unwrap();
///
///     assert!(res.status().is_success());
/// }
/// ```
pub fn start_with<F, I, S, B>(cfg: TestServerConfig, factory: F) -> TestServer
where
    F: Fn() -> I + Send + Clone + 'static,
    I: IntoServiceFactory<S, Request>,
    S: ServiceFactory<Request, Config = AppConfig> + 'static,
    S::Error: Into<Error> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,
    B: MessageBody + 'static,
    B::Error: Into<Box<dyn StdError>>,
{
    let (tx, rx) = mpsc::channel();

    let tls = match cfg.stream {
        StreamType::Tcp => false,
        #[cfg(feature = "openssl")]
        StreamType::Openssl(_) => true,
        #[cfg(feature = "rustls")]
        StreamType::Rustls(_) => true,
    };

    // run server in separate thread
    thread::spawn(move || {
        let sys = rt::System::new();
        let tcp = net::TcpListener::bind("127.0.0.1:0").unwrap();
        let local_addr = tcp.local_addr().unwrap();
        let factory = factory.clone();
        let srv_cfg = cfg.clone();
        let timeout = cfg.client_timeout;
        let builder = Server::build().workers(1).disable_signals();

        let srv = match srv_cfg.stream {
            StreamType::Tcp => match srv_cfg.tp {
                HttpVer::Http1 => builder.listen("test", tcp, move || {
                    let app_cfg =
                        AppConfig::__priv_test_new(false, local_addr.to_string(), local_addr);

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    HttpService::build()
                        .client_timeout(timeout)
                        .h1(map_config(fac, move |_| app_cfg.clone()))
                        .tcp()
                }),
                HttpVer::Http2 => builder.listen("test", tcp, move || {
                    let app_cfg =
                        AppConfig::__priv_test_new(false, local_addr.to_string(), local_addr);

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    HttpService::build()
                        .client_timeout(timeout)
                        .h2(map_config(fac, move |_| app_cfg.clone()))
                        .tcp()
                }),
                HttpVer::Both => builder.listen("test", tcp, move || {
                    let app_cfg =
                        AppConfig::__priv_test_new(false, local_addr.to_string(), local_addr);

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    HttpService::build()
                        .client_timeout(timeout)
                        .finish(map_config(fac, move |_| app_cfg.clone()))
                        .tcp()
                }),
            },
            #[cfg(feature = "openssl")]
            StreamType::Openssl(acceptor) => match cfg.tp {
                HttpVer::Http1 => builder.listen("test", tcp, move || {
                    let app_cfg =
                        AppConfig::__priv_test_new(false, local_addr.to_string(), local_addr);

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    HttpService::build()
                        .client_timeout(timeout)
                        .h1(map_config(fac, move |_| app_cfg.clone()))
                        .openssl(acceptor.clone())
                }),
                HttpVer::Http2 => builder.listen("test", tcp, move || {
                    let app_cfg =
                        AppConfig::__priv_test_new(false, local_addr.to_string(), local_addr);

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    HttpService::build()
                        .client_timeout(timeout)
                        .h2(map_config(fac, move |_| app_cfg.clone()))
                        .openssl(acceptor.clone())
                }),
                HttpVer::Both => builder.listen("test", tcp, move || {
                    let app_cfg =
                        AppConfig::__priv_test_new(false, local_addr.to_string(), local_addr);

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    HttpService::build()
                        .client_timeout(timeout)
                        .finish(map_config(fac, move |_| app_cfg.clone()))
                        .openssl(acceptor.clone())
                }),
            },
            #[cfg(feature = "rustls")]
            StreamType::Rustls(config) => match cfg.tp {
                HttpVer::Http1 => builder.listen("test", tcp, move || {
                    let app_cfg =
                        AppConfig::__priv_test_new(false, local_addr.to_string(), local_addr);

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    HttpService::build()
                        .client_timeout(timeout)
                        .h1(map_config(fac, move |_| app_cfg.clone()))
                        .rustls(config.clone())
                }),
                HttpVer::Http2 => builder.listen("test", tcp, move || {
                    let app_cfg =
                        AppConfig::__priv_test_new(false, local_addr.to_string(), local_addr);

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    HttpService::build()
                        .client_timeout(timeout)
                        .h2(map_config(fac, move |_| app_cfg.clone()))
                        .rustls(config.clone())
                }),
                HttpVer::Both => builder.listen("test", tcp, move || {
                    let app_cfg =
                        AppConfig::__priv_test_new(false, local_addr.to_string(), local_addr);

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    HttpService::build()
                        .client_timeout(timeout)
                        .finish(map_config(fac, move |_| app_cfg.clone()))
                        .rustls(config.clone())
                }),
            },
        }
        .unwrap();

        sys.block_on(async {
            let srv = srv.run();
            tx.send((rt::System::current(), srv, local_addr)).unwrap();
        });

        sys.run()
    });

    let (system, server, addr) = rx.recv().unwrap();

    let client = {
        let connector = {
            #[cfg(feature = "openssl")]
            {
                use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

                let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
                builder.set_verify(SslVerifyMode::NONE);
                let _ = builder
                    .set_alpn_protos(b"\x02h2\x08http/1.1")
                    .map_err(|e| log::error!("Can not set alpn protocol: {:?}", e));
                Connector::new()
                    .conn_lifetime(time::Duration::from_secs(0))
                    .timeout(time::Duration::from_millis(30000))
                    .ssl(builder.build())
            }
            #[cfg(not(feature = "openssl"))]
            {
                Connector::new()
                    .conn_lifetime(time::Duration::from_secs(0))
                    .timeout(time::Duration::from_millis(30000))
            }
        };

        Client::builder().connector(connector).finish()
    };

    TestServer {
        addr,
        client,
        system,
        tls,
        server,
    }
}

#[derive(Debug, Clone)]
enum HttpVer {
    Http1,
    Http2,
    Both,
}

#[derive(Clone)]
enum StreamType {
    Tcp,
    #[cfg(feature = "openssl")]
    Openssl(openssl::ssl::SslAcceptor),
    #[cfg(feature = "rustls")]
    Rustls(rustls::ServerConfig),
}

/// Create default test server config.
pub fn config() -> TestServerConfig {
    TestServerConfig::default()
}

#[derive(Clone)]
pub struct TestServerConfig {
    tp: HttpVer,
    stream: StreamType,
    client_timeout: u64,
}

impl Default for TestServerConfig {
    fn default() -> Self {
        TestServerConfig::new()
    }
}

impl TestServerConfig {
    /// Create default server configuration
    pub(crate) fn new() -> TestServerConfig {
        TestServerConfig {
            tp: HttpVer::Both,
            stream: StreamType::Tcp,
            client_timeout: 5000,
        }
    }

    /// Accept HTTP/1.1 only.
    pub fn h1(mut self) -> Self {
        self.tp = HttpVer::Http1;
        self
    }

    /// Accept HTTP/2 only.
    pub fn h2(mut self) -> Self {
        self.tp = HttpVer::Http2;
        self
    }

    /// Accept secure connections via OpenSSL.
    #[cfg(feature = "openssl")]
    pub fn openssl(mut self, acceptor: openssl::ssl::SslAcceptor) -> Self {
        self.stream = StreamType::Openssl(acceptor);
        self
    }

    /// Accept secure connections via Rustls.
    #[cfg(feature = "rustls")]
    pub fn rustls(mut self, config: rustls::ServerConfig) -> Self {
        self.stream = StreamType::Rustls(config);
        self
    }

    /// Set client timeout in milliseconds for first request.
    pub fn client_timeout(mut self, val: u64) -> Self {
        self.client_timeout = val;
        self
    }
}

/// A basic HTTP server controller that simplifies the process of writing integration tests for
/// Actix Web applications.
///
/// See [`start`] for usage example.
pub struct TestServer {
    addr: net::SocketAddr,
    client: awc::Client,
    system: rt::System,
    tls: bool,
    server: Server,
}

impl TestServer {
    /// Construct test server url
    pub fn addr(&self) -> net::SocketAddr {
        self.addr
    }

    /// Construct test server url
    pub fn url(&self, uri: &str) -> String {
        let scheme = if self.tls { "https" } else { "http" };

        if uri.starts_with('/') {
            format!("{}://localhost:{}{}", scheme, self.addr.port(), uri)
        } else {
            format!("{}://localhost:{}/{}", scheme, self.addr.port(), uri)
        }
    }

    /// Create `GET` request.
    pub fn get(&self, path: impl AsRef<str>) -> ClientRequest {
        self.client.get(self.url(path.as_ref()).as_str())
    }

    /// Create `POST` request.
    pub fn post(&self, path: impl AsRef<str>) -> ClientRequest {
        self.client.post(self.url(path.as_ref()).as_str())
    }

    /// Create `HEAD` request.
    pub fn head(&self, path: impl AsRef<str>) -> ClientRequest {
        self.client.head(self.url(path.as_ref()).as_str())
    }

    /// Create `PUT` request.
    pub fn put(&self, path: impl AsRef<str>) -> ClientRequest {
        self.client.put(self.url(path.as_ref()).as_str())
    }

    /// Create `PATCH` request.
    pub fn patch(&self, path: impl AsRef<str>) -> ClientRequest {
        self.client.patch(self.url(path.as_ref()).as_str())
    }

    /// Create `DELETE` request.
    pub fn delete(&self, path: impl AsRef<str>) -> ClientRequest {
        self.client.delete(self.url(path.as_ref()).as_str())
    }

    /// Create `OPTIONS` request.
    pub fn options(&self, path: impl AsRef<str>) -> ClientRequest {
        self.client.options(self.url(path.as_ref()).as_str())
    }

    /// Connect request with given method and path.
    pub fn request(&self, method: Method, path: impl AsRef<str>) -> ClientRequest {
        self.client.request(method, path.as_ref())
    }

    pub async fn load_body<S>(
        &mut self,
        mut response: ClientResponse<S>,
    ) -> Result<web::Bytes, PayloadError>
    where
        S: Stream<Item = Result<web::Bytes, PayloadError>> + Unpin + 'static,
    {
        response.body().limit(10_485_760).await
    }

    /// Connect to WebSocket server at a given path.
    pub async fn ws_at(
        &mut self,
        path: &str,
    ) -> Result<Framed<impl AsyncRead + AsyncWrite, ws::Codec>, awc::error::WsClientError> {
        let url = self.url(path);
        let connect = self.client.ws(url).connect();
        connect.await.map(|(_, framed)| framed)
    }

    /// Connect to a WebSocket server.
    pub async fn ws(
        &mut self,
    ) -> Result<Framed<impl AsyncRead + AsyncWrite, ws::Codec>, awc::error::WsClientError> {
        self.ws_at("/").await
    }

    /// Get default HeaderMap of Client.
    ///
    /// Returns Some(&mut HeaderMap) when Client object is unique
    /// (No other clone of client exists at the same time).
    pub fn client_headers(&mut self) -> Option<&mut HeaderMap> {
        self.client.headers()
    }

    /// Gracefully stop HTTP server.
    pub async fn stop(self) {
        self.server.handle().stop(true).await;
        self.system.stop();
        rt::time::sleep(time::Duration::from_millis(100)).await;
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.system.stop()
    }
}
