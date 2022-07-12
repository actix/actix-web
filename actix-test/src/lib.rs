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

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]

#[cfg(feature = "openssl")]
extern crate tls_openssl as openssl;
#[cfg(feature = "rustls")]
extern crate tls_rustls as rustls;

use std::{fmt, net, thread, time::Duration};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
pub use actix_http::{body::to_bytes, test::TestBuffer};
use actix_http::{header::HeaderMap, ws, HttpService, Method, Request, Response};
pub use actix_http_test::unused_addr;
use actix_service::{map_config, IntoServiceFactory, ServiceFactory, ServiceFactoryExt as _};
pub use actix_web::test::{
    call_and_read_body, call_and_read_body_json, call_service, init_service, ok_service,
    read_body, read_body_json, status_service, TestRequest,
};
use actix_web::{
    body::MessageBody,
    dev::{AppConfig, Server, ServerHandle, Service},
    rt::{self, System},
    web, Error,
};
use awc::{error::PayloadError, Client, ClientRequest, ClientResponse, Connector};
use futures_core::Stream;
use tokio::sync::mpsc;

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
{
    // for sending handles and server info back from the spawned thread
    let (started_tx, started_rx) = std::sync::mpsc::channel();

    // for signaling the shutdown of spawned server and system
    let (thread_stop_tx, thread_stop_rx) = mpsc::channel(1);

    let tls = match cfg.stream {
        StreamType::Tcp => false,
        #[cfg(feature = "openssl")]
        StreamType::Openssl(_) => true,
        #[cfg(feature = "rustls")]
        StreamType::Rustls(_) => true,
    };

    // run server in separate orphaned thread
    thread::spawn(move || {
        rt::System::new().block_on(async move {
            let tcp = net::TcpListener::bind("127.0.0.1:0").unwrap();
            let local_addr = tcp.local_addr().unwrap();
            let factory = factory.clone();
            let srv_cfg = cfg.clone();
            let timeout = cfg.client_request_timeout;

            let builder = Server::build().workers(1).disable_signals().system_exit();

            let srv = match srv_cfg.stream {
                StreamType::Tcp => match srv_cfg.tp {
                    HttpVer::Http1 => builder.listen("test", tcp, move || {
                        let app_cfg = AppConfig::__priv_test_new(
                            false,
                            local_addr.to_string(),
                            local_addr,
                        );

                        let fac = factory()
                            .into_factory()
                            .map_err(|err| err.into().error_response());

                        HttpService::build()
                            .client_request_timeout(timeout)
                            .h1(map_config(fac, move |_| app_cfg.clone()))
                            .tcp()
                    }),
                    HttpVer::Http2 => builder.listen("test", tcp, move || {
                        let app_cfg = AppConfig::__priv_test_new(
                            false,
                            local_addr.to_string(),
                            local_addr,
                        );

                        let fac = factory()
                            .into_factory()
                            .map_err(|err| err.into().error_response());

                        HttpService::build()
                            .client_request_timeout(timeout)
                            .h2(map_config(fac, move |_| app_cfg.clone()))
                            .tcp()
                    }),
                    HttpVer::Both => builder.listen("test", tcp, move || {
                        let app_cfg = AppConfig::__priv_test_new(
                            false,
                            local_addr.to_string(),
                            local_addr,
                        );

                        let fac = factory()
                            .into_factory()
                            .map_err(|err| err.into().error_response());

                        HttpService::build()
                            .client_request_timeout(timeout)
                            .finish(map_config(fac, move |_| app_cfg.clone()))
                            .tcp()
                    }),
                },
                #[cfg(feature = "openssl")]
                StreamType::Openssl(acceptor) => match cfg.tp {
                    HttpVer::Http1 => builder.listen("test", tcp, move || {
                        let app_cfg = AppConfig::__priv_test_new(
                            false,
                            local_addr.to_string(),
                            local_addr,
                        );

                        let fac = factory()
                            .into_factory()
                            .map_err(|err| err.into().error_response());

                        HttpService::build()
                            .client_request_timeout(timeout)
                            .h1(map_config(fac, move |_| app_cfg.clone()))
                            .openssl(acceptor.clone())
                    }),
                    HttpVer::Http2 => builder.listen("test", tcp, move || {
                        let app_cfg = AppConfig::__priv_test_new(
                            false,
                            local_addr.to_string(),
                            local_addr,
                        );

                        let fac = factory()
                            .into_factory()
                            .map_err(|err| err.into().error_response());

                        HttpService::build()
                            .client_request_timeout(timeout)
                            .h2(map_config(fac, move |_| app_cfg.clone()))
                            .openssl(acceptor.clone())
                    }),
                    HttpVer::Both => builder.listen("test", tcp, move || {
                        let app_cfg = AppConfig::__priv_test_new(
                            false,
                            local_addr.to_string(),
                            local_addr,
                        );

                        let fac = factory()
                            .into_factory()
                            .map_err(|err| err.into().error_response());

                        HttpService::build()
                            .client_request_timeout(timeout)
                            .finish(map_config(fac, move |_| app_cfg.clone()))
                            .openssl(acceptor.clone())
                    }),
                },
                #[cfg(feature = "rustls")]
                StreamType::Rustls(config) => match cfg.tp {
                    HttpVer::Http1 => builder.listen("test", tcp, move || {
                        let app_cfg = AppConfig::__priv_test_new(
                            false,
                            local_addr.to_string(),
                            local_addr,
                        );

                        let fac = factory()
                            .into_factory()
                            .map_err(|err| err.into().error_response());

                        HttpService::build()
                            .client_request_timeout(timeout)
                            .h1(map_config(fac, move |_| app_cfg.clone()))
                            .rustls(config.clone())
                    }),
                    HttpVer::Http2 => builder.listen("test", tcp, move || {
                        let app_cfg = AppConfig::__priv_test_new(
                            false,
                            local_addr.to_string(),
                            local_addr,
                        );

                        let fac = factory()
                            .into_factory()
                            .map_err(|err| err.into().error_response());

                        HttpService::build()
                            .client_request_timeout(timeout)
                            .h2(map_config(fac, move |_| app_cfg.clone()))
                            .rustls(config.clone())
                    }),
                    HttpVer::Both => builder.listen("test", tcp, move || {
                        let app_cfg = AppConfig::__priv_test_new(
                            false,
                            local_addr.to_string(),
                            local_addr,
                        );

                        let fac = factory()
                            .into_factory()
                            .map_err(|err| err.into().error_response());

                        HttpService::build()
                            .client_request_timeout(timeout)
                            .finish(map_config(fac, move |_| app_cfg.clone()))
                            .rustls(config.clone())
                    }),
                },
            }
            .expect("test server could not be created");

            let srv = srv.run();
            started_tx
                .send((System::current(), srv.handle(), local_addr))
                .unwrap();

            // drive server loop
            srv.await.unwrap();

            // notify TestServer that server and system have shut down
            // all thread managed resources should be dropped at this point
        });

        let _ = thread_stop_tx.send(());
    });

    let (system, server, addr) = started_rx.recv().unwrap();

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
                    .conn_lifetime(Duration::from_secs(0))
                    .timeout(Duration::from_millis(30000))
                    .openssl(builder.build())
            }
            #[cfg(not(feature = "openssl"))]
            {
                Connector::new()
                    .conn_lifetime(Duration::from_secs(0))
                    .timeout(Duration::from_millis(30000))
            }
        };

        Client::builder().connector(connector).finish()
    };

    TestServer {
        server,
        thread_stop_rx,
        client,
        system,
        addr,
        tls,
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
    client_request_timeout: Duration,
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
            client_request_timeout: Duration::from_secs(5),
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

    /// Set client timeout for first request.
    pub fn client_request_timeout(mut self, dur: Duration) -> Self {
        self.client_request_timeout = dur;
        self
    }
}

/// A basic HTTP server controller that simplifies the process of writing integration tests for
/// Actix Web applications.
///
/// See [`start`] for usage example.
pub struct TestServer {
    server: ServerHandle,
    thread_stop_rx: mpsc::Receiver<()>,
    client: awc::Client,
    system: rt::System,
    addr: net::SocketAddr,
    tls: bool,
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

    /// Stop HTTP server.
    ///
    /// Waits for spawned `Server` and `System` to shutdown (force) shutdown.
    pub async fn stop(mut self) {
        // signal server to stop
        self.server.stop(false).await;

        // also signal system to stop
        // though this is handled by `ServerBuilder::exit_system` too
        self.system.stop();

        // wait for thread to be stopped but don't care about result
        let _ = self.thread_stop_rx.recv().await;
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // calls in this Drop impl should be enough to shut down the server, system, and thread
        // without needing to await anything

        // signal server to stop
        let _ = self.server.stop(true);

        // signal system to stop
        self.system.stop();
    }
}
