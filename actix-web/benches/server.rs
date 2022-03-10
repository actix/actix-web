use actix_web::{web, App, HttpResponse};
use awc::Client;
use criterion::{criterion_group, criterion_main, Criterion};
use futures_util::future::join_all;

const STR: &str = "Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World";

// benchmark sending all requests at the same time
fn bench_async_burst(c: &mut Criterion) {
    // We are using System here, since Runtime requires preinitialized tokio
    // Maybe add to actix_rt docs
    let rt = actix_rt::System::new();

    let srv = rt.block_on(async {
        actix_test::start(|| {
            App::new().service(
                web::resource("/").route(web::to(|| async { HttpResponse::Ok().body(STR) })),
            )
        })
    });

    let url = srv.url("/");

    c.bench_function("get_body_async_burst", move |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let client = Client::new().get(url.clone()).freeze().unwrap();

                let start = std::time::Instant::now();
                // benchmark body

                let burst = (0..iters).map(|_| client.send());
                let resps = join_all(burst).await;

                let elapsed = start.elapsed();

                // if there are failed requests that might be an issue
                let failed = resps.iter().filter(|r| r.is_err()).count();
                if failed > 0 {
                    eprintln!("failed {} requests (might be bench timeout)", failed);
                };

                elapsed
            })
        })
    });
}

criterion_group!(server_benches, bench_async_burst);
criterion_main!(server_benches);
