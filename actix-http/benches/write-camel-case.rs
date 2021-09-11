use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

fn bench_write_camel_case(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_camel_case");

    let names = ["connection", "Transfer-Encoding", "transfer-encoding"];

    for &i in &names {
        let bts = i.as_bytes();

        group.bench_with_input(BenchmarkId::new("Original", i), bts, |b, bts| {
            b.iter(|| {
                let mut buf = black_box([0; 24]);
                _original::write_camel_case(black_box(bts), &mut buf)
            });
        });

        group.bench_with_input(BenchmarkId::new("New", i), bts, |b, bts| {
            b.iter(|| {
                let mut buf = black_box([0; 24]);
                let len = black_box(bts.len());
                _new::write_camel_case(black_box(bts), buf.as_mut_ptr(), len)
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_write_camel_case);
criterion_main!(benches);

mod _new {
    pub fn write_camel_case(value: &[u8], buf: *mut u8, len: usize) {
        // first copy entire (potentially wrong) slice to output
        let buffer = unsafe {
            std::ptr::copy_nonoverlapping(value.as_ptr(), buf, len);
            std::slice::from_raw_parts_mut(buf, len)
        };

        let mut iter = value.iter();

        // first character should be uppercase
        if let Some(c @ b'a'..=b'z') = iter.next() {
            buffer[0] = c & 0b1101_1111;
        }

        // track 1 ahead of the current position since that's the location being assigned to
        let mut index = 2;

        // remaining characters after hyphens should also be uppercase
        while let Some(&c) = iter.next() {
            if c == b'-' {
                // advance iter by one and uppercase if needed
                if let Some(c @ b'a'..=b'z') = iter.next() {
                    buffer[index] = c & 0b1101_1111;
                }
            }

            index += 1;
        }
    }
}

mod _original {
    pub fn write_camel_case(value: &[u8], buffer: &mut [u8]) {
        let mut index = 0;
        let key = value;
        let mut key_iter = key.iter();

        if let Some(c) = key_iter.next() {
            if *c >= b'a' && *c <= b'z' {
                buffer[index] = *c ^ b' ';
                index += 1;
            }
        } else {
            return;
        }

        while let Some(c) = key_iter.next() {
            buffer[index] = *c;
            index += 1;
            if *c == b'-' {
                if let Some(c) = key_iter.next() {
                    if *c >= b'a' && *c <= b'z' {
                        buffer[index] = *c ^ b' ';
                        index += 1;
                    }
                }
            }
        }
    }
}
