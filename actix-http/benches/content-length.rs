use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use bytes::BytesMut;

// benchmark sending all requests at the same time
fn bench_write_content_length(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_content_length");

    let sizes = [
        0, 1, 11, 83, 101, 653, 1001, 6323, 10001, 56329, 100001, 123456, 98724245,
        4294967202,
    ];

    for i in sizes.iter() {
        group.bench_with_input(BenchmarkId::new("Original (unsafe)", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _original::write_content_length(i, &mut b)
            })
        });

        group.bench_with_input(BenchmarkId::new("New (safe)", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _new::write_content_length(i, &mut b)
            })
        });

        group.bench_with_input(BenchmarkId::new("itoa", i), i, |b, &i| {
            b.iter(|| {
                let mut b = BytesMut::with_capacity(35);
                _itoa::write_content_length(i, &mut b)
            })
        });
    }

    group.finish();
}

criterion_group!(benches, bench_write_content_length);
criterion_main!(benches);

mod _itoa {
    use bytes::{BufMut, BytesMut};

    pub fn write_content_length(n: usize, bytes: &mut BytesMut) {
        if n == 0 {
            bytes.put_slice(b"\r\ncontent-length: 0\r\n");
            return;
        }

        let mut buf = itoa::Buffer::new();

        bytes.put_slice(b"\r\ncontent-length: ");
        bytes.put_slice(buf.format(n).as_bytes());
        bytes.put_slice(b"\r\n");
    }
}

mod _new {
    use bytes::{BufMut, BytesMut};

    const DIGITS_START: u8 = b'0';

    /// NOTE: bytes object has to contain enough space
    pub fn write_content_length(n: usize, bytes: &mut BytesMut) {
        if n == 0 {
            bytes.put_slice(b"\r\ncontent-length: 0\r\n");
            return;
        }

        bytes.put_slice(b"\r\ncontent-length: ");

        if n < 10 {
            bytes.put_u8(DIGITS_START + (n as u8));
        } else if n < 100 {
            let n = n as u8;

            let d10 = n / 10;
            let d1 = n % 10;

            bytes.put_u8(DIGITS_START + d10);
            bytes.put_u8(DIGITS_START + d1);
        } else if n < 1000 {
            let n = n as u16;

            let d100 = (n / 100) as u8;
            let d10 = ((n / 10) % 10) as u8;
            let d1 = (n % 10) as u8;

            bytes.put_u8(DIGITS_START + d100);
            bytes.put_u8(DIGITS_START + d10);
            bytes.put_u8(DIGITS_START + d1);
        } else if n < 10_000 {
            let n = n as u16;

            let d1000 = (n / 1000) as u8;
            let d100 = ((n / 100) % 10) as u8;
            let d10 = ((n / 10) % 10) as u8;
            let d1 = (n % 10) as u8;

            bytes.put_u8(DIGITS_START + d1000);
            bytes.put_u8(DIGITS_START + d100);
            bytes.put_u8(DIGITS_START + d10);
            bytes.put_u8(DIGITS_START + d1);
        } else if n < 100_000 {
            let n = n as u32;

            let d10000 = (n / 10000) as u8;
            let d1000 = ((n / 1000) % 10) as u8;
            let d100 = ((n / 100) % 10) as u8;
            let d10 = ((n / 10) % 10) as u8;
            let d1 = (n % 10) as u8;

            bytes.put_u8(DIGITS_START + d10000);
            bytes.put_u8(DIGITS_START + d1000);
            bytes.put_u8(DIGITS_START + d100);
            bytes.put_u8(DIGITS_START + d10);
            bytes.put_u8(DIGITS_START + d1);
        } else if n < 1_000_000 {
            let n = n as u32;

            let d100000 = (n / 100000) as u8;
            let d10000 = ((n / 10000) % 10) as u8;
            let d1000 = ((n / 1000) % 10) as u8;
            let d100 = ((n / 100) % 10) as u8;
            let d10 = ((n / 10) % 10) as u8;
            let d1 = (n % 10) as u8;

            bytes.put_u8(DIGITS_START + d100000);
            bytes.put_u8(DIGITS_START + d10000);
            bytes.put_u8(DIGITS_START + d1000);
            bytes.put_u8(DIGITS_START + d100);
            bytes.put_u8(DIGITS_START + d10);
            bytes.put_u8(DIGITS_START + d1);
        } else {
            write_usize(n, bytes);
        }

        bytes.put_slice(b"\r\n");
    }

    fn write_usize(n: usize, bytes: &mut BytesMut) {
        let mut n = n;

        // 20 chars is max length of a usize (2^64)
        // digits will be added to the buffer from lsd to msd
        let mut buf = BytesMut::with_capacity(20);

        while n > 9 {
            // "pop" the least-significant digit
            let lsd = (n % 10) as u8;

            // remove the lsd from n
            n = n / 10;

            buf.put_u8(DIGITS_START + lsd);
        }

        // put msd to result buffer
        bytes.put_u8(DIGITS_START + (n as u8));

        // put, in reverse (msd to lsd), remaining digits to buffer
        for i in (0..buf.len()).rev() {
            bytes.put_u8(buf[i]);
        }
    }
}

mod _original {
    use std::{mem, ptr, slice};

    use bytes::{BufMut, BytesMut};

    const DEC_DIGITS_LUT: &[u8] = b"0001020304050607080910111213141516171819\
          2021222324252627282930313233343536373839\
          4041424344454647484950515253545556575859\
          6061626364656667686970717273747576777879\
          8081828384858687888990919293949596979899";

    /// NOTE: bytes object has to contain enough space
    pub fn write_content_length(mut n: usize, bytes: &mut BytesMut) {
        if n < 10 {
            let mut buf: [u8; 21] = [
                b'\r', b'\n', b'c', b'o', b'n', b't', b'e', b'n', b't', b'-', b'l',
                b'e', b'n', b'g', b't', b'h', b':', b' ', b'0', b'\r', b'\n',
            ];
            buf[18] = (n as u8) + b'0';
            bytes.put_slice(&buf);
        } else if n < 100 {
            let mut buf: [u8; 22] = [
                b'\r', b'\n', b'c', b'o', b'n', b't', b'e', b'n', b't', b'-', b'l',
                b'e', b'n', b'g', b't', b'h', b':', b' ', b'0', b'0', b'\r', b'\n',
            ];
            let d1 = n << 1;
            unsafe {
                ptr::copy_nonoverlapping(
                    DEC_DIGITS_LUT.as_ptr().add(d1),
                    buf.as_mut_ptr().offset(18),
                    2,
                );
            }
            bytes.put_slice(&buf);
        } else if n < 1000 {
            let mut buf: [u8; 23] = [
                b'\r', b'\n', b'c', b'o', b'n', b't', b'e', b'n', b't', b'-', b'l',
                b'e', b'n', b'g', b't', b'h', b':', b' ', b'0', b'0', b'0', b'\r',
                b'\n',
            ];
            // decode 2 more chars, if > 2 chars
            let d1 = (n % 100) << 1;
            n /= 100;
            unsafe {
                ptr::copy_nonoverlapping(
                    DEC_DIGITS_LUT.as_ptr().add(d1),
                    buf.as_mut_ptr().offset(19),
                    2,
                )
            };

            // decode last 1
            buf[18] = (n as u8) + b'0';

            bytes.put_slice(&buf);
        } else {
            bytes.put_slice(b"\r\ncontent-length: ");
            convert_usize(n, bytes);
        }
    }

    pub(crate) fn convert_usize(mut n: usize, bytes: &mut BytesMut) {
        let mut curr: isize = 39;
        let mut buf: [u8; 41] = unsafe { mem::MaybeUninit::uninit().assume_init() };
        buf[39] = b'\r';
        buf[40] = b'\n';
        let buf_ptr = buf.as_mut_ptr();
        let lut_ptr = DEC_DIGITS_LUT.as_ptr();

        // eagerly decode 4 characters at a time
        while n >= 10_000 {
            let rem = (n % 10_000) as isize;
            n /= 10_000;

            let d1 = (rem / 100) << 1;
            let d2 = (rem % 100) << 1;
            curr -= 4;
            unsafe {
                ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
                ptr::copy_nonoverlapping(
                    lut_ptr.offset(d2),
                    buf_ptr.offset(curr + 2),
                    2,
                );
            }
        }

        // if we reach here numbers are <= 9999, so at most 4 chars long
        let mut n = n as isize; // possibly reduce 64bit math

        // decode 2 more chars, if > 2 chars
        if n >= 100 {
            let d1 = (n % 100) << 1;
            n /= 100;
            curr -= 2;
            unsafe {
                ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
            }
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
                ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
            }
        }

        unsafe {
            bytes.extend_from_slice(slice::from_raw_parts(
                buf_ptr.offset(curr),
                41 - curr as usize,
            ));
        }
    }
}
