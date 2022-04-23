use criterion::{criterion_group, criterion_main, Criterion};

use bytes::BytesMut;

// A Miri run detects UB, seen on this playground:
// https://play.rust-lang.org/?version=stable&mode=debug&edition=2018&gist=f5d9aa166aa48df8dca05fce2b6c3915

fn bench_header_parsing(c: &mut Criterion) {
    c.bench_function("Original (Unsound) [short]", |b| {
        b.iter(|| {
            let mut buf = BytesMut::from(REQ_SHORT);
            _original::parse_headers(&mut buf);
        })
    });

    c.bench_function("New (safe) [short]", |b| {
        b.iter(|| {
            let mut buf = BytesMut::from(REQ_SHORT);
            _new::parse_headers(&mut buf);
        })
    });

    c.bench_function("Original (Unsound) [realistic]", |b| {
        b.iter(|| {
            let mut buf = BytesMut::from(REQ);
            _original::parse_headers(&mut buf);
        })
    });

    c.bench_function("New (safe) [realistic]", |b| {
        b.iter(|| {
            let mut buf = BytesMut::from(REQ);
            _new::parse_headers(&mut buf);
        })
    });
}

criterion_group!(benches, bench_header_parsing);
criterion_main!(benches);

const MAX_HEADERS: usize = 96;

const EMPTY_HEADER_ARRAY: [httparse::Header<'static>; MAX_HEADERS] =
    [httparse::EMPTY_HEADER; MAX_HEADERS];

#[derive(Clone, Copy)]
struct HeaderIndex {
    name: (usize, usize),
    value: (usize, usize),
}

const EMPTY_HEADER_INDEX: HeaderIndex = HeaderIndex {
    name: (0, 0),
    value: (0, 0),
};

const EMPTY_HEADER_INDEX_ARRAY: [HeaderIndex; MAX_HEADERS] = [EMPTY_HEADER_INDEX; MAX_HEADERS];

impl HeaderIndex {
    fn record(bytes: &[u8], headers: &[httparse::Header<'_>], indices: &mut [HeaderIndex]) {
        let bytes_ptr = bytes.as_ptr() as usize;
        for (header, indices) in headers.iter().zip(indices.iter_mut()) {
            let name_start = header.name.as_ptr() as usize - bytes_ptr;
            let name_end = name_start + header.name.len();
            indices.name = (name_start, name_end);
            let value_start = header.value.as_ptr() as usize - bytes_ptr;
            let value_end = value_start + header.value.len();
            indices.value = (value_start, value_end);
        }
    }
}

// test cases taken from:
// https://github.com/seanmonstar/httparse/blob/master/benches/parse.rs

const REQ_SHORT: &[u8] = b"\
GET / HTTP/1.0\r\n\
Host: example.com\r\n\
Cookie: session=60; user_id=1\r\n\r\n";

const REQ: &[u8] = b"\
GET /wp-content/uploads/2010/03/hello-kitty-darth-vader-pink.jpg HTTP/1.1\r\n\
Host: www.kittyhell.com\r\n\
User-Agent: Mozilla/5.0 (Macintosh; U; Intel Mac OS X 10.6; ja-JP-mac; rv:1.9.2.3) Gecko/20100401 Firefox/3.6.3 Pathtraq/0.9\r\n\
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8\r\n\
Accept-Language: ja,en-us;q=0.7,en;q=0.3\r\n\
Accept-Encoding: gzip,deflate\r\n\
Accept-Charset: Shift_JIS,utf-8;q=0.7,*;q=0.7\r\n\
Keep-Alive: 115\r\n\
Connection: keep-alive\r\n\
Cookie: wp_ozh_wsa_visits=2; wp_ozh_wsa_visit_lasttime=xxxxxxxxxx; __utma=xxxxxxxxx.xxxxxxxxxx.xxxxxxxxxx.xxxxxxxxxx.xxxxxxxxxx.x; __utmz=xxxxxxxxx.xxxxxxxxxx.x.x.utmccn=(referral)|utmcsr=reader.livedoor.com|utmcct=/reader/|utmcmd=referral|padding=under256\r\n\r\n";

mod _new {
    use super::*;

    pub fn parse_headers(src: &mut BytesMut) -> usize {
        let mut headers: [HeaderIndex; MAX_HEADERS] = EMPTY_HEADER_INDEX_ARRAY;
        let mut parsed: [httparse::Header<'_>; MAX_HEADERS] = EMPTY_HEADER_ARRAY;

        let mut req = httparse::Request::new(&mut parsed);
        match req.parse(src).unwrap() {
            httparse::Status::Complete(_len) => {
                HeaderIndex::record(src, req.headers, &mut headers);
                req.headers.len()
            }
            _ => unreachable!(),
        }
    }
}

mod _original {
    use super::*;

    use std::mem::MaybeUninit;

    pub fn parse_headers(src: &mut BytesMut) -> usize {
        #![allow(invalid_value, clippy::uninit_assumed_init)]

        let mut headers: [HeaderIndex; MAX_HEADERS] =
            unsafe { MaybeUninit::uninit().assume_init() };

        #[allow(invalid_value)]
        let mut parsed: [httparse::Header<'_>; MAX_HEADERS] =
            unsafe { MaybeUninit::uninit().assume_init() };

        let mut req = httparse::Request::new(&mut parsed);
        match req.parse(src).unwrap() {
            httparse::Status::Complete(_len) => {
                HeaderIndex::record(src, req.headers, &mut headers);
                req.headers.len()
            }
            _ => unreachable!(),
        }
    }
}
