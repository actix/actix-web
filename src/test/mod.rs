//! Various helpers for Actix applications to use during testing.

use std::{net, thread};
use std::sync::mpsc;

use actix::{Arbiter, SyncAddress, System, msgs};
use tokio_core::net::TcpListener;

use server::HttpServer;
use handler::Handler;
use channel::IntoHttpHandler;
use middlewares::Middleware;
use application::{Application, HttpApplication};


/// The `TestServer` type.
///
/// `TestServer` is very simple test server that simplify process of writing
/// integrational tests cases for actix web applications.
///
/// # Examples
///
/// ```rust
/// # extern crate actix;
/// # extern crate actix_web;
/// # use actix_web::*;
/// # extern crate reqwest;
/// #
/// # fn my_handler(req: HttpRequest) -> HttpResponse {
/// #     httpcodes::HTTPOk.response()
/// # }
/// #
/// # fn main() {
/// use actix_web::test::TestServer;
///
/// let srv = TestServer::new(|app| app.handler(my_handler));
///
/// assert!(reqwest::get(&srv.url("/")).unwrap().status().is_success());
/// # }
/// ```
pub struct TestServer {
    addr: net::SocketAddr,
    thread: Option<thread::JoinHandle<()>>,
    sys: SyncAddress<System>,
}

impl TestServer {

    /// Start new test server
    ///
    /// This methos accepts configuration method. You can add
    /// middlewares or set handlers for test application.
    pub fn new<F>(config: F) -> Self
        where F: Sync + Send + 'static + Fn(&mut TestApp<()>),
    {
        TestServer::with_state(||(), config)
    }

    /// Start new test server with custom application state
    ///
    /// This methos accepts state factory and configuration method.
    pub fn with_state<S, FS, F>(state: FS, config: F) -> Self
        where S: 'static,
              FS: Sync + Send + 'static + Fn() -> S,
              F: Sync + Send + 'static + Fn(&mut TestApp<S>),
    {
        let (tx, rx) = mpsc::channel();

        // run server in separate thread
        let join = thread::spawn(move || {
            let sys = System::new("actix-test-server");

            let tcp = net::TcpListener::bind("0.0.0.0:0").unwrap();
            let local_addr = tcp.local_addr().unwrap();
            let tcp = TcpListener::from_listener(tcp, &local_addr, Arbiter::handle()).unwrap();

            HttpServer::new(move || {
                let mut app = TestApp::new(state());
                config(&mut app);
                app}
            ).start_incoming(tcp.incoming(), false);

            tx.send((Arbiter::system(), local_addr)).unwrap();
            let _ = sys.run();
        });

        let (sys, addr) = rx.recv().unwrap();
        TestServer {
            addr: addr,
            thread: Some(join),
            sys: sys,
        }
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
            self.sys.send(msgs::SystemExit(0));
            let _ = handle.join();
        }
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

    fn into_handler(self) -> HttpApplication<S> {
        self.app.unwrap().finish()
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
