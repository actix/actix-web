#![allow(clippy::uninlined_format_args)]

use actix_http::{body, encoding::Encoder, ContentEncoding, ResponseHead, StatusCode};
use criterion::{criterion_group, criterion_main, Criterion};

const BODY: &[u8] = include_bytes!("../../Cargo.lock");

const CHUNK_SIZES: [usize; 7] = [512, 1024, 2048, 4096, 8192, 16384, 32768];

const CONTENT_ENCODING: [ContentEncoding; 4] = [
    ContentEncoding::Deflate,
    ContentEncoding::Gzip,
    ContentEncoding::Zstd,
    ContentEncoding::Brotli,
];

fn compression_responses(c: &mut Criterion) {
    static_assertions::const_assert!(BODY.len() > CHUNK_SIZES[6]);

    let mut group = c.benchmark_group("time to compress chunk");

    for content_encoding in CONTENT_ENCODING {
        for chunk_size in CHUNK_SIZES {
            group.bench_function(
                format!("{}-{}", content_encoding.as_str(), chunk_size),
                |b| {
                    let rt = actix_rt::Runtime::new().unwrap();
                    b.iter(|| {
                        rt.block_on(async move {
                            let encoder = Encoder::response(
                                content_encoding,
                                &mut ResponseHead::new(StatusCode::OK),
                                &BODY[..chunk_size],
                            )
                            .with_encode_chunk_size(chunk_size);
                            body::to_bytes_limited(encoder, chunk_size + 256)
                                .await
                                .unwrap()
                                .unwrap();
                        });
                    });
                },
            );
        }
    }

    group.finish();
}

criterion_group!(benches, compression_responses);
criterion_main!(benches);
