use actix_service::Service;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::{web, App, Error, HttpResponse};
use criterion::{criterion_main, Criterion};
use std::cell::RefCell;
use std::rc::Rc;

use actix_web::test::{init_service, ok_service, TestRequest};

/// Criterion Benchmark for async Service
/// Should be used from within criterion group:
/// ```rust,ignore
/// let mut criterion: ::criterion::Criterion<_> =
///     ::criterion::Criterion::default().configure_from_args();
/// bench_async_service(&mut criterion, ok_service(), "async_service_direct");
/// ```
///
/// Usable for benching Service wrappers:
/// Using minimum service code implementation we first measure
/// time to run minimum service, then measure time with wrapper.
///
/// Sample output
/// async_service_direct    time:   [1.0908 us 1.1656 us 1.2613 us]
pub fn bench_async_service<S>(c: &mut Criterion, srv: S, name: &str)
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse, Error = Error>
        + 'static,
{
    let mut rt = actix_rt::System::new("test");
    let srv = Rc::new(RefCell::new(srv));

    let req = TestRequest::default().to_srv_request();
    assert!(rt
        .block_on(srv.borrow_mut().call(req))
        .unwrap()
        .status()
        .is_success());

    // start benchmark loops
    c.bench_function(name, move |b| {
        b.iter_custom(|iters| {
            let srv = srv.clone();
            // exclude request generation, it appears it takes significant time vs call (3us vs 1us)
            let reqs: Vec<_> = (0..iters)
                .map(|_| TestRequest::default().to_srv_request())
                .collect();
            let start = std::time::Instant::now();
            // benchmark body
            rt.block_on(async move {
                for req in reqs {
                    srv.borrow_mut().call(req).await.unwrap();
                }
            });
            let elapsed = start.elapsed();
            // check that at least first request succeeded
            elapsed
        })
    });
}

async fn index(req: ServiceRequest) -> Result<ServiceResponse, Error> {
    Ok(req.into_response(HttpResponse::Ok().finish()))
}

// Benchmark basic WebService directly
// this approach is usable for benching WebService, though it adds some time to direct service call:
// Sample results on MacBook Pro '14
// time:   [2.0724 us 2.1345 us 2.2074 us]
fn async_web_service(c: &mut Criterion) {
    let mut rt = actix_rt::System::new("test");
    let srv = Rc::new(RefCell::new(rt.block_on(init_service(
        App::new().service(web::service("/").finish(index)),
    ))));

    let req = TestRequest::get().uri("/").to_request();
    assert!(rt
        .block_on(srv.borrow_mut().call(req))
        .unwrap()
        .status()
        .is_success());

    // start benchmark loops
    c.bench_function("async_web_service_direct", move |b| {
        b.iter_custom(|iters| {
            let srv = srv.clone();
            let reqs = (0..iters).map(|_| TestRequest::get().uri("/").to_request());

            let start = std::time::Instant::now();
            // benchmark body
            rt.block_on(async move {
                for req in reqs {
                    srv.borrow_mut().call(req).await.unwrap();
                }
            });
            let elapsed = start.elapsed();
            // check that at least first request succeeded
            elapsed
        })
    });
}

pub fn service_benches() {
    let mut criterion: ::criterion::Criterion<_> =
        ::criterion::Criterion::default().configure_from_args();
    bench_async_service(&mut criterion, ok_service(), "async_service_direct");
    async_web_service(&mut criterion);
}
criterion_main!(service_benches);
