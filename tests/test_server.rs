extern crate actix;
extern crate actix_web;
extern crate tokio_core;
extern crate reqwest;

use std::{net, thread};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_core::net::TcpListener;

use actix::*;
use actix_web::*;

#[test]
fn test_serve() {
    thread::spawn(|| {
        let sys = System::new("test");
        let srv = HttpServer::new(
            vec![Application::new("/")
                 .resource("/", |r| r.method(Method::GET).h(httpcodes::HTTPOk))]);
        srv.serve::<_, ()>("127.0.0.1:58902").unwrap();
        sys.run();
    });
    assert!(reqwest::get("http://localhost:58902/").unwrap().status().is_success());
}

#[test]
fn test_serve_incoming() {
    let loopback = net::Ipv4Addr::new(127, 0, 0, 1);
    let socket = net::SocketAddrV4::new(loopback, 0);
    let tcp = net::TcpListener::bind(socket).unwrap();
    let addr1 = tcp.local_addr().unwrap();
    let addr2 = tcp.local_addr().unwrap();

    thread::spawn(move || {
        let sys = System::new("test");

        let srv = HttpServer::new(
            Application::new("/")
                .resource("/", |r| r.method(Method::GET).h(httpcodes::HTTPOk)));
        let tcp = TcpListener::from_listener(tcp, &addr2, Arbiter::handle()).unwrap();
        srv.serve_incoming::<_, ()>(tcp.incoming(), false).unwrap();
        sys.run();
    });

    assert!(reqwest::get(&format!("http://{}/", addr1))
            .unwrap().status().is_success());
}

struct MiddlewareTest {
    start: Arc<AtomicUsize>,
    response: Arc<AtomicUsize>,
    finish: Arc<AtomicUsize>,
}

impl<S> middlewares::Middleware<S> for MiddlewareTest {
    fn start(&self, _: &mut HttpRequest<S>) -> middlewares::Started {
        self.start.store(self.start.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        middlewares::Started::Done
    }

    fn response(&self, _: &mut HttpRequest<S>, resp: HttpResponse) -> middlewares::Response {
        self.response.store(self.response.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        middlewares::Response::Done(resp)
    }

    fn finish(&self, _: &mut HttpRequest<S>, _: &HttpResponse) -> middlewares::Finished {
        self.finish.store(self.finish.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        middlewares::Finished::Done
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
            vec![Application::new("/")
                 .middleware(MiddlewareTest{start: act_num1,
                                            response: act_num2,
                                            finish: act_num3})
                 .resource("/", |r| r.method(Method::GET).h(httpcodes::HTTPOk))
                 .finish()])
            .serve::<_, ()>("127.0.0.1:58904").unwrap();
        sys.run();
    });

    assert!(reqwest::get("http://localhost:58904/").unwrap().status().is_success());
    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}
