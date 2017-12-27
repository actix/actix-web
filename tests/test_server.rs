extern crate actix;
extern crate actix_web;
extern crate tokio_core;
extern crate reqwest;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use actix_web::*;

#[test]
fn test_serve() {
    let srv = test::TestServer::new(|app| app.handler(httpcodes::HTTPOk));
    assert!(reqwest::get(&srv.url("/")).unwrap().status().is_success());
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
