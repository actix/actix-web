extern crate actix;
extern crate actix_web;
extern crate tokio_core;
extern crate reqwest;

use std::{net, thread};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_core::net::TcpListener;

use actix::*;
use actix_web::*;

fn create_server<T, A>() -> HttpServer<T, A, Application<()>> {
    HttpServer::new(
        vec![Application::default("/")
             .resource("/", |r|
                       r.handler(Method::GET, |_, _, _| {
                           httpcodes::HTTPOk
                       }))
             .finish()])
}

#[test]
fn test_serve() {
    thread::spawn(|| {
        let sys = System::new("test");
        let srv = create_server();
        srv.serve::<_, ()>("127.0.0.1:58902").unwrap();
        sys.run();
    });
    assert!(reqwest::get("http://localhost:58906/").unwrap().status().is_success());
}

#[test]
fn test_serve_incoming() {
    thread::spawn(|| {
        let sys = System::new("test");

        let srv = create_server();
        let addr = net::SocketAddr::from_str("127.0.0.1:58906").unwrap();
        let tcp = TcpListener::bind(&addr, Arbiter::handle()).unwrap();
        srv.serve_incoming::<_, ()>(tcp.incoming()).unwrap();
        sys.run();

    });

    assert!(reqwest::get("http://localhost:58906/").unwrap().status().is_success());
}

struct MiddlewareTest {
    start: Arc<AtomicUsize>,
    response: Arc<AtomicUsize>,
    finish: Arc<AtomicUsize>,
}

impl Middleware for MiddlewareTest {
    fn start(&self, _: &mut HttpRequest) -> Result<(), HttpResponse> {
        self.start.store(self.start.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        Ok(())
    }

    fn response(&self, _: &mut HttpRequest, resp: HttpResponse) -> HttpResponse {
        self.response.store(self.response.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        resp
    }

    fn finish(&self, _: &mut HttpRequest, _: &HttpResponse) {
        self.finish.store(self.finish.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
    }
}

#[test]
fn test_middlewares() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    thread::spawn(move || {
        let sys = System::new("test");

        HttpServer::new(
            vec![Application::default("/")
                 .middleware(MiddlewareTest{start: act_num1,
                                            response: act_num2,
                                            finish: act_num3})
                 .resource("/", |r|
                           r.handler(Method::GET, |_, _, _| {
                               httpcodes::HTTPOk
                           }))
                 .finish()])
            .serve::<_, ()>("127.0.0.1:58903").unwrap();
        sys.run();
    });

    assert!(reqwest::get("http://localhost:58903/").unwrap().status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}
