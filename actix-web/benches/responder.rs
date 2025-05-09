use std::{future::Future, time::Instant};

use actix_http::body::BoxBody;
use actix_utils::future::{ready, Ready};
use actix_web::{http::StatusCode, test::TestRequest, Error, HttpRequest, HttpResponse, Responder};
use criterion::{criterion_group, criterion_main, Criterion};
use futures_util::future::join_all;

// responder simulate the old responder trait.
trait FutureResponder {
    type Error;
    type Future: Future<Output = Result<HttpResponse, Self::Error>>;

    fn future_respond_to(self, req: &HttpRequest) -> Self::Future;
}

// a simple wrapper type around string
struct StringResponder(String);

impl FutureResponder for StringResponder {
    type Error = Error;
    type Future = Ready<Result<HttpResponse, Self::Error>>;

    fn future_respond_to(self, _: &HttpRequest) -> Self::Future {
        // this is default builder for string response in both new and old responder trait.
        ready(Ok(HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self.0)))
    }
}

impl Responder for StringResponder {
    type Body = BoxBody;

    fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self.0)
    }
}

fn future_responder(c: &mut Criterion) {
    let rt = actix_rt::System::new();
    let req = TestRequest::default().to_http_request();

    c.bench_function("future_responder", move |b| {
        b.iter_custom(|_| {
            let futs = (0..100_000).map(|_| async {
                StringResponder(String::from("Hello World!!"))
                    .future_respond_to(&req)
                    .await
            });

            let futs = join_all(futs);

            let start = Instant::now();

            let _res = rt.block_on(futs);

            start.elapsed()
        })
    });
}

fn responder(c: &mut Criterion) {
    let rt = actix_rt::System::new();
    let req = TestRequest::default().to_http_request();
    c.bench_function("responder", move |b| {
        b.iter_custom(|_| {
            let responders = (0..100_000).map(|_| StringResponder(String::from("Hello World!!")));

            let start = Instant::now();
            let _res = rt.block_on(async {
                // don't need runtime block on but to be fair.
                responders.map(|r| r.respond_to(&req)).collect::<Vec<_>>()
            });

            start.elapsed()
        })
    });
}

criterion_group!(responder_bench, future_responder, responder);
criterion_main!(responder_bench);
