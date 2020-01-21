mod experiments;
mod service;

use actix_web::test::ok_service;
use criterion::{criterion_main, Criterion};
use experiments::cloneable::cloneable;
use experiments::cloneable::cloneable_safe;
use service::bench_async_service;

// This is benchmark of effect by replacing UnsafeCell to RefCell in CloneableService
// Issue: https://github.com/actix/actix-web/issues/1295
//
// Note: numbers may vary from run to run +-20%, probably due to async env
// async_service_direct    time:   [1.0076 us 1.0300 us 1.0507 us]
//                         change: [-32.491% -23.295% -15.790%] (p = 0.00 < 0.05)
// async_service_cloneable_unsafe
//                         time:   [1.0857 us 1.1208 us 1.1629 us]
//                         change: [-2.9318% +5.7660% +15.004%] (p = 0.27 > 0.05)
// async_service_cloneable_safe
//                         time:   [1.0703 us 1.1002 us 1.1390 us]
//                         change: [-9.2951% -1.1186% +6.5384%] (p = 0.80 > 0.05)

pub fn service_benches() {
    let mut criterion: Criterion<_> = Criterion::default().configure_from_args();
    bench_async_service(&mut criterion, ok_service(), "async_service_direct");
    bench_async_service(
        &mut criterion,
        cloneable::CloneableService::new(ok_service()),
        "async_service_cloneable_unsafe",
    );
    bench_async_service(
        &mut criterion,
        cloneable_safe::CloneableService::new(ok_service()),
        "async_service_cloneable_safe",
    );
}
criterion_main!(service_benches);
