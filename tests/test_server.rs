extern crate actix;
extern crate actix_web;
extern crate tokio_core;
extern crate reqwest;
extern crate futures;
extern crate h2;
extern crate http;

use std::{net, thread, time};
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicUsize, Ordering};
use futures::Future;
use h2::client;
use http::Request;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;

use actix_web::*;
use actix::System;

#[test]
fn test_start() {
    let _ = test::TestServer::unused_addr();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let sys = System::new("test");
        let srv = HttpServer::new(
            || vec![Application::new()
                    .resource("/", |r| r.method(Method::GET).h(httpcodes::HTTPOk))]);

        let srv = srv.bind("127.0.0.1:0").unwrap();
        let addr = srv.addrs()[0];
        let srv_addr = srv.start();
        let _ = tx.send((addr, srv_addr));
        sys.run();
    });
    let (addr, srv_addr) = rx.recv().unwrap();
    assert!(reqwest::get(&format!("http://{}/", addr)).unwrap().status().is_success());

    // pause
    let _ = srv_addr.call_fut(dev::PauseServer).wait();
    thread::sleep(time::Duration::from_millis(100));
    assert!(net::TcpStream::connect(addr).is_err());

    // resume
    let _ = srv_addr.call_fut(dev::ResumeServer).wait();
    assert!(reqwest::get(&format!("http://{}/", addr)).unwrap().status().is_success());
}

#[test]
fn test_simple() {
    let srv = test::TestServer::new(|app| app.handler(httpcodes::HTTPOk));
    assert!(reqwest::get(&srv.url("/")).unwrap().status().is_success());
}

#[test]
fn test_h2() {
    let srv = test::TestServer::new(|app| app.handler(httpcodes::HTTPOk));
    let addr = srv.addr();

    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let tcp = TcpStream::connect(&addr, &handle);

    let tcp = tcp.then(|res| {
        client::handshake(res.unwrap())
    }).then(move |res| {
        let (mut client, h2) = res.unwrap();

        let request = Request::builder()
            .uri(format!("https://{}/", addr).as_str())
            .body(())
            .unwrap();
        let (response, _) = client.send_request(request, false).unwrap();

        // Spawn a task to run the conn...
        handle.spawn(h2.map_err(|e| println!("GOT ERR={:?}", e)));

        response
    });
    let resp = core.run(tcp).unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[test]
fn test_application() {
    let srv = test::TestServer::with_factory(
        || Application::new().resource("/", |r| r.h(httpcodes::HTTPOk)));
    assert!(reqwest::get(&srv.url("/")).unwrap().status().is_success());
}

struct MiddlewareTest {
    start: Arc<AtomicUsize>,
    response: Arc<AtomicUsize>,
    finish: Arc<AtomicUsize>,
}

impl<S> middleware::Middleware<S> for MiddlewareTest {
    fn start(&self, _: &mut HttpRequest<S>) -> middleware::Started {
        self.start.store(self.start.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        middleware::Started::Done
    }

    fn response(&self, _: &mut HttpRequest<S>, resp: HttpResponse) -> middleware::Response {
        self.response.store(self.response.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        middleware::Response::Done(resp)
    }

    fn finish(&self, _: &mut HttpRequest<S>, _: &HttpResponse) -> middleware::Finished {
        self.finish.store(self.finish.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        middleware::Finished::Done
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

    let srv = test::TestServer::new(
        move |app| app.middleware(MiddlewareTest{start: Arc::clone(&act_num1),
                                                 response: Arc::clone(&act_num2),
                                                 finish: Arc::clone(&act_num3)})
            .handler(httpcodes::HTTPOk)
    );
    
    assert!(reqwest::get(&srv.url("/")).unwrap().status().is_success());
    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}


#[test]
fn test_resource_middlewares() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let srv = test::TestServer::new(
        move |app| app.handler2(
            httpcodes::HTTPOk,
            MiddlewareTest{start: Arc::clone(&act_num1),
                           response: Arc::clone(&act_num2),
                           finish: Arc::clone(&act_num3)})
    );

    assert!(reqwest::get(&srv.url("/")).unwrap().status().is_success());
    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    // assert_eq!(num3.load(Ordering::Relaxed), 1);
}
