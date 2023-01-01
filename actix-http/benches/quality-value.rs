#![allow(clippy::uninlined_format_args)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

const CODES: &[u16] = &[0, 1000, 201, 800, 550];

fn bench_quality_display_impls(c: &mut Criterion) {
    let mut group = c.benchmark_group("quality value display impls");

    for i in CODES.iter() {
        group.bench_with_input(BenchmarkId::new("New (fast?)", i), i, |b, &i| {
            b.iter(|| _new::Quality(i).to_string())
        });

        group.bench_with_input(BenchmarkId::new("Naive", i), i, |b, &i| {
            b.iter(|| _naive::Quality(i).to_string())
        });
    }

    group.finish();
}

criterion_group!(benches, bench_quality_display_impls);
criterion_main!(benches);

mod _new {
    use std::fmt;

    pub struct Quality(pub(crate) u16);

    impl fmt::Display for Quality {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self.0 {
                0 => f.write_str("0"),
                1000 => f.write_str("1"),

                // some number in the range 1–999
                x => {
                    f.write_str("0.")?;

                    // this implementation avoids string allocation otherwise required
                    // for `.trim_end_matches('0')`

                    if x < 10 {
                        f.write_str("00")?;
                        // 0 is handled so it's not possible to have a trailing 0, we can just return
                        itoa_fmt(f, x)
                    } else if x < 100 {
                        f.write_str("0")?;
                        if x % 10 == 0 {
                            // trailing 0, divide by 10 and write
                            itoa_fmt(f, x / 10)
                        } else {
                            itoa_fmt(f, x)
                        }
                    } else {
                        // x is in range 101–999

                        if x % 100 == 0 {
                            // two trailing 0s, divide by 100 and write
                            itoa_fmt(f, x / 100)
                        } else if x % 10 == 0 {
                            // one trailing 0, divide by 10 and write
                            itoa_fmt(f, x / 10)
                        } else {
                            itoa_fmt(f, x)
                        }
                    }
                }
            }
        }
    }

    pub fn itoa_fmt<W: fmt::Write, V: itoa::Integer>(mut wr: W, value: V) -> fmt::Result {
        let mut buf = itoa::Buffer::new();
        wr.write_str(buf.format(value))
    }
}

mod _naive {
    use std::fmt;

    pub struct Quality(pub(crate) u16);

    impl fmt::Display for Quality {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self.0 {
                0 => f.write_str("0"),
                1000 => f.write_str("1"),

                x => {
                    write!(f, "{}", format!("{:03}", x).trim_end_matches('0'))
                }
            }
        }
    }
}
