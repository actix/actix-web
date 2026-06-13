#![no_main]
use libfuzzer_sys::fuzz_target;
use actix_http::header::HeaderMap;
use http::header::{HeaderName, HeaderValue};

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 { return; }
    let split = data[0] as usize % data.len().min(64);
    let (name, value) = if split < data.len() { 
        (&data[..split.min(data.len())], &data[split.min(data.len())..])
    } else {
        (data, &[] as &[u8])
    };
    let _ = HeaderName::from_bytes(name).map(|n| {
        let _ = HeaderValue::from_bytes(value).map(|v| {
            let mut map = HeaderMap::new();
            map.insert(n, v);
        });
    });
});
