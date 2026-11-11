use std::hint::black_box;

use actix_codec::Decoder as _;
use actix_http::{
    h1::{Codec, Message},
    HttpMessage as _,
};
use bytes::BytesMut;

const REQUEST: &[u8] = concat!(
    "GET /api/items?page=1&limit=20 HTTP/1.1\r\n",
    "Host: www.example.com\r\n",
    "User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36\r\n",
    "Accept: application/json, text/plain, */*\r\n",
    "Accept-Language: en-GB,en;q=0.9\r\n",
    "Accept-Encoding: gzip, deflate, br\r\n",
    "Connection: keep-alive\r\n",
    "Referer: https://www.example.com/dashboard\r\n",
    "Origin: https://www.example.com\r\n",
    "Cookie: session=0123456789abcdef0123456789abcdef; theme=dark\r\n",
    "Sec-Fetch-Dest: empty\r\n",
    "Sec-Fetch-Mode: cors\r\n",
    "Sec-Fetch-Site: same-origin\r\n",
    "\r\n",
).as_bytes();

#[divan::bench(args = [1, REQUEST.len()])]
fn request_12_headers(b: divan::Bencher<'_, '_>, chunk_size: usize) {
    let mut codec = Codec::default();
    let mut buf = BytesMut::with_capacity(REQUEST.len());

    b.bench_local(|| {
        for chunk in black_box(REQUEST).chunks(chunk_size) {
            buf.extend_from_slice(chunk);

            if let Some(message) = black_box(codec.decode(&mut buf).unwrap()) {
                let Message::Item(req) = message else {
                    panic!("unexpected payload")
                };
                assert_eq!(req.headers().len(), 12);
                black_box(req);
            }
        }

        assert!(buf.is_empty());
    });
}

fn main() {
    actix_rt::System::new().block_on(async { divan::main() });
}
