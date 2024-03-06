use std::time::SystemTime;

use actix_http::header::HttpDate;
use divan::{black_box, AllocProfiler, Bencher};

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

#[divan::bench]
fn date_formatting(b: Bencher<'_, '_>) {
    let now = SystemTime::now();

    b.bench(|| {
        black_box(HttpDate::from(black_box(now)).to_string());
    })
}

fn main() {
    divan::main();
}
