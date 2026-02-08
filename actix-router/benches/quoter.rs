use std::{borrow::Cow, fmt::Write as _};

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn compare_quoters(c: &mut Criterion) {
    let mut group = c.benchmark_group("Compare Quoters");

    let quoter = actix_router::Quoter::new(b"", b"");
    let path_quoted = (0..=0x7f).fold(String::new(), |mut buf, c| {
        write!(&mut buf, "%{:02X}", c).unwrap();
        buf
    });
    let path_unquoted = ('\u{00}'..='\u{7f}').collect::<String>();

    group.bench_function("quoter_unquoted", |b| {
        b.iter(|| {
            for _ in 0..10 {
                black_box(quoter.requote(path_unquoted.as_bytes()));
            }
        });
    });

    group.bench_function("percent_encode_unquoted", |b| {
        b.iter(|| {
            for _ in 0..10 {
                let decode = percent_encoding::percent_decode(path_unquoted.as_bytes());
                black_box(Into::<Cow<'_, [u8]>>::into(decode));
            }
        });
    });

    group.bench_function("quoter_quoted", |b| {
        b.iter(|| {
            for _ in 0..10 {
                black_box(quoter.requote(path_quoted.as_bytes()));
            }
        });
    });

    group.bench_function("percent_encode_quoted", |b| {
        b.iter(|| {
            for _ in 0..10 {
                let decode = percent_encoding::percent_decode(path_quoted.as_bytes());
                black_box(Into::<Cow<'_, [u8]>>::into(decode));
            }
        });
    });

    group.finish();
}

criterion_group!(benches, compare_quoters);
criterion_main!(benches);
