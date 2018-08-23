extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate tokio_timer;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use actix_web::error::{Error, ErrorInternalServerError};
use actix_web::*;
use futures::{future, Future};
use tokio_timer::Delay;

struct MiddlewareTest {
    start: Arc<AtomicUsize>,
    response: Arc<AtomicUsize>,
    finish: Arc<AtomicUsize>,
}

impl<S> middleware::Middleware<S> for MiddlewareTest {
    fn start(&self, _: &HttpRequest<S>) -> Result<middleware::Started> {
        self.start
            .store(self.start.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        Ok(middleware::Started::Done)
    }

    fn response(
        &self, _: &HttpRequest<S>, resp: HttpResponse,
    ) -> Result<middleware::Response> {
        self.response
            .store(self.response.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        Ok(middleware::Response::Done(resp))
    }

    fn finish(&self, _: &HttpRequest<S>, _: &HttpResponse) -> middleware::Finished {
        self.finish
            .store(self.finish.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        middleware::Finished::Done
    }
}

#[test]
fn test_middleware() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::new(move |app| {
        app.middleware(MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        }).handler(|_| HttpResponse::Ok())
    });

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_middleware_multiple() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::new(move |app| {
        app.middleware(MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        }).middleware(MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        }).handler(|_| HttpResponse::Ok())
    });

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 2);
    assert_eq!(num2.load(Ordering::Relaxed), 2);
    assert_eq!(num3.load(Ordering::Relaxed), 2);
}

#[test]
fn test_resource_middleware() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::new(move |app| {
        app.middleware(MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        }).handler(|_| HttpResponse::Ok())
    });

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_resource_middleware_multiple() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::new(move |app| {
        app.middleware(MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        }).middleware(MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        }).handler(|_| HttpResponse::Ok())
    });

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 2);
    assert_eq!(num2.load(Ordering::Relaxed), 2);
    assert_eq!(num3.load(Ordering::Relaxed), 2);
}

#[test]
fn test_scope_middleware() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        App::new().scope("/scope", |scope| {
            scope
                .middleware(MiddlewareTest {
                    start: Arc::clone(&act_num1),
                    response: Arc::clone(&act_num2),
                    finish: Arc::clone(&act_num3),
                }).resource("/test", |r| r.f(|_| HttpResponse::Ok()))
        })
    });

    let request = srv.get().uri(srv.url("/scope/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_scope_middleware_multiple() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        App::new().scope("/scope", |scope| {
            scope
                .middleware(MiddlewareTest {
                    start: Arc::clone(&act_num1),
                    response: Arc::clone(&act_num2),
                    finish: Arc::clone(&act_num3),
                }).middleware(MiddlewareTest {
                    start: Arc::clone(&act_num1),
                    response: Arc::clone(&act_num2),
                    finish: Arc::clone(&act_num3),
                }).resource("/test", |r| r.f(|_| HttpResponse::Ok()))
        })
    });

    let request = srv.get().uri(srv.url("/scope/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 2);
    assert_eq!(num2.load(Ordering::Relaxed), 2);
    assert_eq!(num3.load(Ordering::Relaxed), 2);
}

#[test]
fn test_middleware_async_handler() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        App::new()
            .middleware(MiddlewareAsyncTest {
                start: Arc::clone(&act_num1),
                response: Arc::clone(&act_num2),
                finish: Arc::clone(&act_num3),
            }).resource("/", |r| {
                r.route().a(|_| {
                    Delay::new(Instant::now() + Duration::from_millis(10))
                        .and_then(|_| Ok(HttpResponse::Ok()))
                })
            })
    });

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    thread::sleep(Duration::from_millis(20));
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_resource_middleware_async_handler() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        let mw = MiddlewareAsyncTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        App::new().resource("/test", |r| {
            r.middleware(mw);
            r.route().a(|_| {
                Delay::new(Instant::now() + Duration::from_millis(10))
                    .and_then(|_| Ok(HttpResponse::Ok()))
            })
        })
    });

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_scope_middleware_async_handler() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        App::new().scope("/scope", |scope| {
            scope
                .middleware(MiddlewareAsyncTest {
                    start: Arc::clone(&act_num1),
                    response: Arc::clone(&act_num2),
                    finish: Arc::clone(&act_num3),
                }).resource("/test", |r| {
                    r.route().a(|_| {
                        Delay::new(Instant::now() + Duration::from_millis(10))
                            .and_then(|_| Ok(HttpResponse::Ok()))
                    })
                })
        })
    });

    let request = srv.get().uri(srv.url("/scope/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

fn index_test_middleware_async_error(_: &HttpRequest) -> FutureResponse<HttpResponse> {
    future::result(Err(error::ErrorBadRequest("TEST"))).responder()
}

#[test]
fn test_middleware_async_error() {
    let req = Arc::new(AtomicUsize::new(0));
    let resp = Arc::new(AtomicUsize::new(0));
    let fin = Arc::new(AtomicUsize::new(0));

    let act_req = Arc::clone(&req);
    let act_resp = Arc::clone(&resp);
    let act_fin = Arc::clone(&fin);

    let mut srv = test::TestServer::new(move |app| {
        app.middleware(MiddlewareTest {
            start: Arc::clone(&act_req),
            response: Arc::clone(&act_resp),
            finish: Arc::clone(&act_fin),
        }).handler(index_test_middleware_async_error)
    });

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    assert_eq!(req.load(Ordering::Relaxed), 1);
    assert_eq!(resp.load(Ordering::Relaxed), 1);
    assert_eq!(fin.load(Ordering::Relaxed), 1);
}

#[test]
fn test_scope_middleware_async_error() {
    let req = Arc::new(AtomicUsize::new(0));
    let resp = Arc::new(AtomicUsize::new(0));
    let fin = Arc::new(AtomicUsize::new(0));

    let act_req = Arc::clone(&req);
    let act_resp = Arc::clone(&resp);
    let act_fin = Arc::clone(&fin);

    let mut srv = test::TestServer::with_factory(move || {
        App::new().scope("/scope", |scope| {
            scope
                .middleware(MiddlewareAsyncTest {
                    start: Arc::clone(&act_req),
                    response: Arc::clone(&act_resp),
                    finish: Arc::clone(&act_fin),
                }).resource("/test", |r| r.f(index_test_middleware_async_error))
        })
    });

    let request = srv.get().uri(srv.url("/scope/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    assert_eq!(req.load(Ordering::Relaxed), 1);
    assert_eq!(resp.load(Ordering::Relaxed), 1);
    assert_eq!(fin.load(Ordering::Relaxed), 1);
}

#[test]
fn test_resource_middleware_async_error() {
    let req = Arc::new(AtomicUsize::new(0));
    let resp = Arc::new(AtomicUsize::new(0));
    let fin = Arc::new(AtomicUsize::new(0));

    let act_req = Arc::clone(&req);
    let act_resp = Arc::clone(&resp);
    let act_fin = Arc::clone(&fin);

    let mut srv = test::TestServer::with_factory(move || {
        let mw = MiddlewareAsyncTest {
            start: Arc::clone(&act_req),
            response: Arc::clone(&act_resp),
            finish: Arc::clone(&act_fin),
        };

        App::new().resource("/test", move |r| {
            r.middleware(mw);
            r.f(index_test_middleware_async_error);
        })
    });

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    assert_eq!(req.load(Ordering::Relaxed), 1);
    assert_eq!(resp.load(Ordering::Relaxed), 1);
    assert_eq!(fin.load(Ordering::Relaxed), 1);
}

struct MiddlewareAsyncTest {
    start: Arc<AtomicUsize>,
    response: Arc<AtomicUsize>,
    finish: Arc<AtomicUsize>,
}

impl<S> middleware::Middleware<S> for MiddlewareAsyncTest {
    fn start(&self, _: &HttpRequest<S>) -> Result<middleware::Started> {
        let to = Delay::new(Instant::now() + Duration::from_millis(10));

        let start = Arc::clone(&self.start);
        Ok(middleware::Started::Future(Box::new(
            to.from_err().and_then(move |_| {
                start.fetch_add(1, Ordering::Relaxed);
                Ok(None)
            }),
        )))
    }

    fn response(
        &self, _: &HttpRequest<S>, resp: HttpResponse,
    ) -> Result<middleware::Response> {
        let to = Delay::new(Instant::now() + Duration::from_millis(10));

        let response = Arc::clone(&self.response);
        Ok(middleware::Response::Future(Box::new(
            to.from_err().and_then(move |_| {
                response.fetch_add(1, Ordering::Relaxed);
                Ok(resp)
            }),
        )))
    }

    fn finish(&self, _: &HttpRequest<S>, _: &HttpResponse) -> middleware::Finished {
        let to = Delay::new(Instant::now() + Duration::from_millis(10));

        let finish = Arc::clone(&self.finish);
        middleware::Finished::Future(Box::new(to.from_err().and_then(move |_| {
            finish.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })))
    }
}

#[test]
fn test_async_middleware() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::new(move |app| {
        app.middleware(MiddlewareAsyncTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        }).handler(|_| HttpResponse::Ok())
    });

    let request = srv.get().finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);

    thread::sleep(Duration::from_millis(20));
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_async_middleware_multiple() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        App::new()
            .middleware(MiddlewareAsyncTest {
                start: Arc::clone(&act_num1),
                response: Arc::clone(&act_num2),
                finish: Arc::clone(&act_num3),
            }).middleware(MiddlewareAsyncTest {
                start: Arc::clone(&act_num1),
                response: Arc::clone(&act_num2),
                finish: Arc::clone(&act_num3),
            }).resource("/test", |r| r.f(|_| HttpResponse::Ok()))
    });

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 2);
    assert_eq!(num2.load(Ordering::Relaxed), 2);

    thread::sleep(Duration::from_millis(50));
    assert_eq!(num3.load(Ordering::Relaxed), 2);
}

#[test]
fn test_async_sync_middleware_multiple() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        App::new()
            .middleware(MiddlewareAsyncTest {
                start: Arc::clone(&act_num1),
                response: Arc::clone(&act_num2),
                finish: Arc::clone(&act_num3),
            }).middleware(MiddlewareTest {
                start: Arc::clone(&act_num1),
                response: Arc::clone(&act_num2),
                finish: Arc::clone(&act_num3),
            }).resource("/test", |r| r.f(|_| HttpResponse::Ok()))
    });

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 2);
    assert_eq!(num2.load(Ordering::Relaxed), 2);

    thread::sleep(Duration::from_millis(50));
    assert_eq!(num3.load(Ordering::Relaxed), 2);
}

#[test]
fn test_async_scope_middleware() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        App::new().scope("/scope", |scope| {
            scope
                .middleware(MiddlewareAsyncTest {
                    start: Arc::clone(&act_num1),
                    response: Arc::clone(&act_num2),
                    finish: Arc::clone(&act_num3),
                }).resource("/test", |r| r.f(|_| HttpResponse::Ok()))
        })
    });

    let request = srv.get().uri(srv.url("/scope/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);

    thread::sleep(Duration::from_millis(20));
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_async_scope_middleware_multiple() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        App::new().scope("/scope", |scope| {
            scope
                .middleware(MiddlewareAsyncTest {
                    start: Arc::clone(&act_num1),
                    response: Arc::clone(&act_num2),
                    finish: Arc::clone(&act_num3),
                }).middleware(MiddlewareAsyncTest {
                    start: Arc::clone(&act_num1),
                    response: Arc::clone(&act_num2),
                    finish: Arc::clone(&act_num3),
                }).resource("/test", |r| r.f(|_| HttpResponse::Ok()))
        })
    });

    let request = srv.get().uri(srv.url("/scope/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 2);
    assert_eq!(num2.load(Ordering::Relaxed), 2);

    thread::sleep(Duration::from_millis(20));
    assert_eq!(num3.load(Ordering::Relaxed), 2);
}

#[test]
fn test_async_async_scope_middleware_multiple() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        App::new().scope("/scope", |scope| {
            scope
                .middleware(MiddlewareAsyncTest {
                    start: Arc::clone(&act_num1),
                    response: Arc::clone(&act_num2),
                    finish: Arc::clone(&act_num3),
                }).middleware(MiddlewareTest {
                    start: Arc::clone(&act_num1),
                    response: Arc::clone(&act_num2),
                    finish: Arc::clone(&act_num3),
                }).resource("/test", |r| r.f(|_| HttpResponse::Ok()))
        })
    });

    let request = srv.get().uri(srv.url("/scope/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 2);
    assert_eq!(num2.load(Ordering::Relaxed), 2);

    thread::sleep(Duration::from_millis(20));
    assert_eq!(num3.load(Ordering::Relaxed), 2);
}

#[test]
fn test_async_resource_middleware() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        let mw = MiddlewareAsyncTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        App::new().resource("/test", move |r| {
            r.middleware(mw);
            r.f(|_| HttpResponse::Ok());
        })
    });

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);

    thread::sleep(Duration::from_millis(40));
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_async_resource_middleware_multiple() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        let mw1 = MiddlewareAsyncTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        let mw2 = MiddlewareAsyncTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        App::new().resource("/test", move |r| {
            r.middleware(mw1);
            r.middleware(mw2);
            r.f(|_| HttpResponse::Ok());
        })
    });

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 2);
    assert_eq!(num2.load(Ordering::Relaxed), 2);

    thread::sleep(Duration::from_millis(40));
    assert_eq!(num3.load(Ordering::Relaxed), 2);
}

#[test]
fn test_async_sync_resource_middleware_multiple() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        let mw1 = MiddlewareAsyncTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        let mw2 = MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        App::new().resource("/test", move |r| {
            r.middleware(mw1);
            r.middleware(mw2);
            r.f(|_| HttpResponse::Ok());
        })
    });

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();
    assert!(response.status().is_success());

    assert_eq!(num1.load(Ordering::Relaxed), 2);
    assert_eq!(num2.load(Ordering::Relaxed), 2);

    thread::sleep(Duration::from_millis(40));
    assert_eq!(num3.load(Ordering::Relaxed), 2);
}

struct MiddlewareWithErr;

impl<S> middleware::Middleware<S> for MiddlewareWithErr {
    fn start(&self, _: &HttpRequest<S>) -> Result<middleware::Started, Error> {
        Err(ErrorInternalServerError("middleware error"))
    }
}

struct MiddlewareAsyncWithErr;

impl<S> middleware::Middleware<S> for MiddlewareAsyncWithErr {
    fn start(&self, _: &HttpRequest<S>) -> Result<middleware::Started, Error> {
        Ok(middleware::Started::Future(Box::new(future::err(
            ErrorInternalServerError("middleware error"),
        ))))
    }
}

#[test]
fn test_middleware_chain_with_error() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        let mw1 = MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        App::new()
            .middleware(mw1)
            .middleware(MiddlewareWithErr)
            .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
    });

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    srv.execute(request.send()).unwrap();

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_middleware_async_chain_with_error() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        let mw1 = MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        App::new()
            .middleware(mw1)
            .middleware(MiddlewareAsyncWithErr)
            .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
    });

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    srv.execute(request.send()).unwrap();

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_scope_middleware_chain_with_error() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        let mw1 = MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        App::new().scope("/scope", |scope| {
            scope
                .middleware(mw1)
                .middleware(MiddlewareWithErr)
                .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
        })
    });

    let request = srv.get().uri(srv.url("/scope/test")).finish().unwrap();
    srv.execute(request.send()).unwrap();

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_scope_middleware_async_chain_with_error() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        let mw1 = MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        App::new().scope("/scope", |scope| {
            scope
                .middleware(mw1)
                .middleware(MiddlewareAsyncWithErr)
                .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
        })
    });

    let request = srv.get().uri(srv.url("/scope/test")).finish().unwrap();
    srv.execute(request.send()).unwrap();

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_resource_middleware_chain_with_error() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        let mw1 = MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        App::new().resource("/test", move |r| {
            r.middleware(mw1);
            r.middleware(MiddlewareWithErr);
            r.f(|_| HttpResponse::Ok());
        })
    });

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    srv.execute(request.send()).unwrap();

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[test]
fn test_resource_middleware_async_chain_with_error() {
    let num1 = Arc::new(AtomicUsize::new(0));
    let num2 = Arc::new(AtomicUsize::new(0));
    let num3 = Arc::new(AtomicUsize::new(0));

    let act_num1 = Arc::clone(&num1);
    let act_num2 = Arc::clone(&num2);
    let act_num3 = Arc::clone(&num3);

    let mut srv = test::TestServer::with_factory(move || {
        let mw1 = MiddlewareTest {
            start: Arc::clone(&act_num1),
            response: Arc::clone(&act_num2),
            finish: Arc::clone(&act_num3),
        };
        App::new().resource("/test", move |r| {
            r.middleware(mw1);
            r.middleware(MiddlewareAsyncWithErr);
            r.f(|_| HttpResponse::Ok());
        })
    });

    let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    srv.execute(request.send()).unwrap();

    assert_eq!(num1.load(Ordering::Relaxed), 1);
    assert_eq!(num2.load(Ordering::Relaxed), 1);
    assert_eq!(num3.load(Ordering::Relaxed), 1);
}

#[cfg(feature = "session")]
#[test]
fn test_session_storage_middleware() {
    use actix_web::middleware::session::{
        CookieSessionBackend, RequestSession, SessionStorage,
    };

    const SIMPLE_NAME: &'static str = "simple";
    const SIMPLE_PAYLOAD: &'static str = "kantan";
    const COMPLEX_NAME: &'static str = "test";
    const COMPLEX_PAYLOAD: &'static str = "url=https://test.com&generate_204";
    //TODO: investigate how to handle below input
    //const COMPLEX_PAYLOAD: &'static str = "FJc%26continue_url%3Dhttp%253A%252F%252Fconnectivitycheck.gstatic.com%252Fgenerate_204";

    let mut srv = test::TestServer::with_factory(move || {
        App::new()
            .middleware(SessionStorage::new(
                CookieSessionBackend::signed(&[0; 32]).secure(false),
            )).resource("/index", move |r| {
                r.f(|req| {
                    let res = req.session().set(COMPLEX_NAME, COMPLEX_PAYLOAD);
                    assert!(res.is_ok());
                    let value = req.session().get::<String>(COMPLEX_NAME);
                    assert!(value.is_ok());
                    let value = value.unwrap();
                    assert!(value.is_some());
                    assert_eq!(value.unwrap(), COMPLEX_PAYLOAD);

                    let res = req.session().set(SIMPLE_NAME, SIMPLE_PAYLOAD);
                    assert!(res.is_ok());
                    let value = req.session().get::<String>(SIMPLE_NAME);
                    assert!(value.is_ok());
                    let value = value.unwrap();
                    assert!(value.is_some());
                    assert_eq!(value.unwrap(), SIMPLE_PAYLOAD);

                    HttpResponse::Ok()
                })
            }).resource("/expect_cookie", move |r| {
                r.f(|req| {
                    let _cookies = req.cookies().expect("To get cookies");

                    let value = req.session().get::<String>(SIMPLE_NAME);
                    assert!(value.is_ok());
                    let value = value.unwrap();
                    assert!(value.is_some());
                    assert_eq!(value.unwrap(), SIMPLE_PAYLOAD);

                    let value = req.session().get::<String>(COMPLEX_NAME);
                    assert!(value.is_ok());
                    let value = value.unwrap();
                    assert!(value.is_some());
                    assert_eq!(value.unwrap(), COMPLEX_PAYLOAD);

                    HttpResponse::Ok()
                })
            })
    });

    let request = srv.get().uri(srv.url("/index")).finish().unwrap();
    let response = srv.execute(request.send()).unwrap();

    assert!(response.headers().contains_key("set-cookie"));
    let set_cookie = response.headers().get("set-cookie");
    assert!(set_cookie.is_some());
    let set_cookie = set_cookie.unwrap().to_str().expect("Convert to str");

    let request = srv
        .get()
        .uri(srv.url("/expect_cookie"))
        .header("cookie", set_cookie.split(';').next().unwrap())
        .finish()
        .unwrap();

    srv.execute(request.send()).unwrap();
}
