#![no_main]
use actix_http::ws;
use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;
use tokio_util::codec::Decoder;

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let mut codec = ws::Codec::new().max_size(1 << 16);
    if mode & 1 == 1 {
        codec = codec.client_mode();
    }
    let step = ((mode >> 1) as usize % 32) + 1;
    let mut buf = BytesMut::new();
    for chunk in rest.chunks(step) {
        buf.extend_from_slice(chunk);
        loop {
            match codec.decode(&mut buf) {
                Ok(Some(_)) => continue,
                Ok(None) => break,
                Err(_) => return,
            }
        }
    }
});
