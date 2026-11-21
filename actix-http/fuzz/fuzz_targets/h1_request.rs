#![no_main]
use actix_http::{h1, ServiceConfig};
use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;
use tokio_util::codec::Decoder;

// ServiceConfig spawns a date-refresh task, so it needs a runtime and a LocalSet.
// Build both once per process and keep them entered.
thread_local! {
    static CONFIG: ServiceConfig = {
        let rt = Box::leak(Box::new(
            tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap(),
        ));
        let local = Box::leak(Box::new(tokio::task::LocalSet::new()));
        std::mem::forget(rt.enter());
        std::mem::forget(local.enter());
        ServiceConfig::default()
    };
}

// First byte picks how the input is split into reads, to exercise
// partial-buffer parsing the way a real socket delivers data.
fuzz_target!(|data: &[u8]| {
    let Some((&step, rest)) = data.split_first() else {
        return;
    };
    let step = (step as usize % 64) + 1;
    let mut codec = CONFIG.with(|c| h1::Codec::new(c.clone()));
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
    let _ = codec.decode_eof(&mut buf);
});
