use std::convert::Infallible;

use actix_http::{encoding::Encoder, ContentEncoding, Request, Response, StatusCode};
use actix_service::{fn_service, Service as _};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

static BODY: &[u8] = include_bytes!("../Cargo.toml");

fn compression_responses(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression responses");

    group.bench_function("identity", |b| {
        let rt = actix_rt::Runtime::new().unwrap();

        let identity_svc = fn_service(|_: Request| async move {
            let mut res = Response::with_body(StatusCode::OK, ());
            let body = black_box(Encoder::response(
                ContentEncoding::Identity,
                res.head_mut(),
                BODY,
            ));
            Ok::<_, Infallible>(black_box(res.set_body(black_box(body))))
        });

        b.iter(|| {
            rt.block_on(identity_svc.call(Request::new())).unwrap();
        });
    });

    group.bench_function("gzip", |b| {
        let rt = actix_rt::Runtime::new().unwrap();

        let identity_svc = fn_service(|_: Request| async move {
            let mut res = Response::with_body(StatusCode::OK, ());
            let body = black_box(Encoder::response(
                ContentEncoding::Gzip,
                res.head_mut(),
                BODY,
            ));
            Ok::<_, Infallible>(black_box(res.set_body(black_box(body))))
        });

        b.iter(|| {
            rt.block_on(identity_svc.call(Request::new())).unwrap();
        });
    });

    group.bench_function("br", |b| {
        let rt = actix_rt::Runtime::new().unwrap();

        let identity_svc = fn_service(|_: Request| async move {
            let mut res = Response::with_body(StatusCode::OK, ());
            let body = black_box(Encoder::response(
                ContentEncoding::Brotli,
                res.head_mut(),
                BODY,
            ));
            Ok::<_, Infallible>(black_box(res.set_body(black_box(body))))
        });

        b.iter(|| {
            rt.block_on(identity_svc.call(Request::new())).unwrap();
        });
    });

    group.bench_function("zstd", |b| {
        let rt = actix_rt::Runtime::new().unwrap();

        let identity_svc = fn_service(|_: Request| async move {
            let mut res = Response::with_body(StatusCode::OK, ());
            let body = black_box(Encoder::response(
                ContentEncoding::Zstd,
                res.head_mut(),
                BODY,
            ));
            Ok::<_, Infallible>(black_box(res.set_body(black_box(body))))
        });

        b.iter(|| {
            rt.block_on(identity_svc.call(Request::new())).unwrap();
        });
    });

    group.finish();
}

criterion_group!(benches, compression_responses);
criterion_main!(benches);
