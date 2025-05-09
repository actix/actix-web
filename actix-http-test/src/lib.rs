//! Various helpers for Actix applications to use during testing.

#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

#[cfg(feature = "openssl")]
extern crate tls_openssl as openssl;

use std::{net, thread, time::Duration};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_rt::{net::TcpStream, System};
use actix_server::{Server, ServerServiceFactory};
use awc::{
    error::PayloadError, http::header::HeaderMap, ws, Client, ClientRequest, ClientResponse,
    Connector,
};
use bytes::Bytes;
use futures_core::stream::Stream;
use http::Method;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::sync::mpsc;

/// Start test server.
///
/// `TestServer` is very simple test server that simplify process of writing integration tests cases
/// for HTTP applications.
///
/// # Examples
///
/// ```
/// use actix_http::{HttpService, Response, Error, StatusCode};
/// use actix_http_test::test_server;
/// use actix_service::{fn_service, map_config, ServiceFactoryExt as _};
///
/// #[actix_rt::test]
/// # async fn hidden_test() {}
/// async fn test_example() {
///     let srv = test_server(|| {
///         HttpService::build()
///             .h1(fn_service(|req| async move {
///                 Ok::<_, Error>(Response::ok())
///             }))
///             .tcp()
///             .map_err(|_| ())
///     })
///     .await;
///
///     let req = srv.get("/");
///     let response = req.send().await.unwrap();
///
///     assert_eq!(response.status(), StatusCode::OK);
/// }
/// # actix_rt::System::new().block_on(test_example());
/// ```
pub async fn test_server<F: ServerServiceFactory<TcpStream>>(factory: F) -> TestServer {
    let tcp = net::TcpListener::bind("127.0.0.1:0").unwrap();
    test_server_with_addr(tcp, factory).await
}

/// Start [`test server`](test_server()) on an existing address binding.
pub async fn test_server_with_addr<F: ServerServiceFactory<TcpStream>>(
    tcp: net::TcpListener,
    factory: F,
) -> TestServer {
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (thread_stop_tx, thread_stop_rx) = mpsc::channel(1);

    // run server in separate thread
    thread::spawn(move || {
        System::new().block_on(async move {
            let local_addr = tcp.local_addr().unwrap();

            let srv = Server::build()
                .workers(1)
                .disable_signals()
                .system_exit()
                .listen("test", tcp, factory)
                .expect("test server could not be created");

            let srv = srv.run();
            started_tx
                .send((System::current(), srv.handle(), local_addr))
                .unwrap();

            // drive server loop
            srv.await.unwrap();
        });

        // notify TestServer that server and system have shut down
        // all thread managed resources should be dropped at this point
        #[allow(clippy::let_underscore_future)]
        let _ = thread_stop_tx.send(());
    });

    let (system, server, addr) = started_rx.recv().unwrap();

    let client = {
        #[cfg(feature = "openssl")]
        let connector = {
            use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

            let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();

            builder.set_verify(SslVerifyMode::NONE);
            let _ = builder
                .set_alpn_protos(b"\x02h2\x08http/1.1")
                .map_err(|err| log::error!("Can not set ALPN protocol: {err}"));

            Connector::new()
                .conn_lifetime(Duration::from_secs(0))
                .timeout(Duration::from_millis(30000))
                .openssl(builder.build())
        };

        #[cfg(not(feature = "openssl"))]
        let connector = {
            Connector::new()
                .conn_lifetime(Duration::from_secs(0))
                .timeout(Duration::from_millis(30000))
        };

        Client::builder().connector(connector).finish()
    };

    TestServer {
        server,
        client,
        system,
        addr,
        thread_stop_rx,
    }
}

/// Test server controller
pub struct TestServer {
    server: actix_server::ServerHandle,
    client: awc::Client,
    system: actix_rt::System,
    addr: net::SocketAddr,
    thread_stop_rx: mpsc::Receiver<()>,
}

impl TestServer {
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

    /// Construct test HTTPS server URL.
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

    /// Create HTTPS `GET` request
    pub fn sget<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.get(self.surl(path.as_ref()).as_str())
    }

    /// Create `POST` request
    pub fn post<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.post(self.url(path.as_ref()).as_str())
    }

    /// Create HTTPS `POST` request
    pub fn spost<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.post(self.surl(path.as_ref()).as_str())
    }

    /// Create `HEAD` request
    pub fn head<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.head(self.url(path.as_ref()).as_str())
    }

    /// Create HTTPS `HEAD` request
    pub fn shead<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.head(self.surl(path.as_ref()).as_str())
    }

    /// Create `PUT` request
    pub fn put<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.put(self.url(path.as_ref()).as_str())
    }

    /// Create HTTPS `PUT` request
    pub fn sput<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.put(self.surl(path.as_ref()).as_str())
    }

    /// Create `PATCH` request
    pub fn patch<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.patch(self.url(path.as_ref()).as_str())
    }

    /// Create HTTPS `PATCH` request
    pub fn spatch<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.patch(self.surl(path.as_ref()).as_str())
    }

    /// Create `DELETE` request
    pub fn delete<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.delete(self.url(path.as_ref()).as_str())
    }

    /// Create HTTPS `DELETE` request
    pub fn sdelete<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.delete(self.surl(path.as_ref()).as_str())
    }

    /// Create `OPTIONS` request
    pub fn options<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.options(self.url(path.as_ref()).as_str())
    }

    /// Create HTTPS `OPTIONS` request
    pub fn soptions<S: AsRef<str>>(&self, path: S) -> ClientRequest {
        self.client.options(self.surl(path.as_ref()).as_str())
    }

    /// Connect to test HTTP server
    pub fn request<S: AsRef<str>>(&self, method: Method, path: S) -> ClientRequest {
        self.client.request(method, path.as_ref())
    }

    pub async fn load_body<S>(
        &mut self,
        mut response: ClientResponse<S>,
    ) -> Result<Bytes, PayloadError>
    where
        S: Stream<Item = Result<Bytes, PayloadError>> + Unpin + 'static,
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
    /// Waits for spawned `Server` and `System` to (force) shutdown.
    pub async fn stop(&mut self) {
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
        #[allow(clippy::let_underscore_future)]
        let _ = self.server.stop(true);

        // signal system to stop
        self.system.stop();
    }
}

/// Get a localhost socket address with random, unused port.
pub fn unused_addr() -> net::SocketAddr {
    let addr: net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    socket.bind(&addr.into()).unwrap();
    socket.set_reuse_address(true).unwrap();
    let tcp = net::TcpListener::from(socket);
    tcp.local_addr().unwrap()
}
