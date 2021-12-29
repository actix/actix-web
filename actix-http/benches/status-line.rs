use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use bytes::BytesMut;
use http::Version;

const CODES: &[u16] = &[201, 303, 404, 515];

fn bench_write_status_line_11(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_status_line v1.1");

    let version = Version::HTTP_11;

    for i in CODES.iter() {
        group.bench_with_input(BenchmarkId::new("Original (unsafe)", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _original::write_status_line(version, i, &mut b);
            })
        });

        group.bench_with_input(BenchmarkId::new("New (safe)", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _new::write_status_line(version, i, &mut b);
            })
        });

        group.bench_with_input(BenchmarkId::new("Naive", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _naive::write_status_line(version, i, &mut b);
            })
        });
    }

    group.finish();
}

fn bench_write_status_line_10(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_status_line v1.0");

    let version = Version::HTTP_10;

    for i in CODES.iter() {
        group.bench_with_input(BenchmarkId::new("Original (unsafe)", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _original::write_status_line(version, i, &mut b);
            })
        });

        group.bench_with_input(BenchmarkId::new("New (safe)", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _new::write_status_line(version, i, &mut b);
            })
        });

        group.bench_with_input(BenchmarkId::new("Naive", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _naive::write_status_line(version, i, &mut b);
            })
        });
    }

    group.finish();
}

fn bench_write_status_line_09(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_status_line v0.9");

    let version = Version::HTTP_09;

    for i in CODES.iter() {
        group.bench_with_input(BenchmarkId::new("Original (unsafe)", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _original::write_status_line(version, i, &mut b);
            })
        });

        group.bench_with_input(BenchmarkId::new("New (safe)", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _new::write_status_line(version, i, &mut b);
            })
        });

        group.bench_with_input(BenchmarkId::new("Naive", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _naive::write_status_line(version, i, &mut b);
            })
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_write_status_line_11,
    bench_write_status_line_10,
    bench_write_status_line_09
);
criterion_main!(benches);

mod _naive {
    use bytes::{BufMut, BytesMut};
    use http::Version;

    pub(crate) fn write_status_line(version: Version, n: u16, bytes: &mut BytesMut) {
        match version {
            Version::HTTP_11 => bytes.put_slice(b"HTTP/1.1 "),
            Version::HTTP_10 => bytes.put_slice(b"HTTP/1.0 "),
            Version::HTTP_09 => bytes.put_slice(b"HTTP/0.9 "),
            _ => {
                // other HTTP version handlers do not use this method
            }
        }

        bytes.put_slice(n.to_string().as_bytes());
    }
}

mod _new {
    use bytes::{BufMut, BytesMut};
    use http::Version;

    const DIGITS_START: u8 = b'0';

    pub(crate) fn write_status_line(version: Version, n: u16, bytes: &mut BytesMut) {
        match version {
            Version::HTTP_11 => bytes.put_slice(b"HTTP/1.1 "),
            Version::HTTP_10 => bytes.put_slice(b"HTTP/1.0 "),
            Version::HTTP_09 => bytes.put_slice(b"HTTP/0.9 "),
            _ => {
                // other HTTP version handlers do not use this method
            }
        }

        let d100 = (n / 100) as u8;
        let d10 = ((n / 10) % 10) as u8;
        let d1 = (n % 10) as u8;

        bytes.put_u8(DIGITS_START + d100);
        bytes.put_u8(DIGITS_START + d10);
        bytes.put_u8(DIGITS_START + d1);

        bytes.put_u8(b' ');
    }
}

mod _original {
    use std::ptr;

    use bytes::{BufMut, BytesMut};
    use http::Version;

    const DEC_DIGITS_LUT: &[u8] = b"0001020304050607080910111213141516171819\
        2021222324252627282930313233343536373839\
        4041424344454647484950515253545556575859\
        6061626364656667686970717273747576777879\
        8081828384858687888990919293949596979899";

    pub(crate) const STATUS_LINE_BUF_SIZE: usize = 13;

    pub(crate) fn write_status_line(version: Version, mut n: u16, bytes: &mut BytesMut) {
        let mut buf: [u8; STATUS_LINE_BUF_SIZE] = *b"HTTP/1.1     ";

        match version {
            Version::HTTP_2 => buf[5] = b'2',
            Version::HTTP_10 => buf[7] = b'0',
            Version::HTTP_09 => {
                buf[5] = b'0';
                buf[7] = b'9';
            }
            _ => {}
        }

        let mut curr: isize = 12;
        let buf_ptr = buf.as_mut_ptr();
        let lut_ptr = DEC_DIGITS_LUT.as_ptr();
        let four = n > 999;

        // decode 2 more chars, if > 2 chars
        let d1 = (n % 100) << 1;
        n /= 100;
        curr -= 2;
        unsafe {
            ptr::copy_nonoverlapping(lut_ptr.offset(d1 as isize), buf_ptr.offset(curr), 2);
        }

        // decode last 1 or 2 chars
        if n < 10 {
            curr -= 1;
            unsafe {
                *buf_ptr.offset(curr) = (n as u8) + b'0';
            }
        } else {
            let d1 = n << 1;
            curr -= 2;
            unsafe {
                ptr::copy_nonoverlapping(lut_ptr.offset(d1 as isize), buf_ptr.offset(curr), 2);
            }
        }

        bytes.put_slice(&buf);
        if four {
            bytes.put_u8(b' ');
        }
    }
}
