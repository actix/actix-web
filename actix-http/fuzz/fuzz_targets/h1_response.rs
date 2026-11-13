#![no_main]
use actix_http::{
    h1::{self, ClientCodec, ClientPayloadCodec, MessageType},
    ServiceConfig,
};
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

// The client codec is two-phase, like awc uses it: decode a response head, then,
// if the head says there is a body, switch to the payload codec until it reports
// the end of the body, then switch back for the next response on the connection.
enum State {
    Head(ClientCodec),
    Body(ClientPayloadCodec),
}

/// Decodes as much of `buf` as possible. Returns `None` once the input is rejected.
fn drive(mut state: State, buf: &mut BytesMut) -> Option<State> {
    loop {
        state = match state {
            State::Head(mut codec) => match codec.decode(buf) {
                Ok(Some(_)) if codec.message_type() == MessageType::None => State::Head(codec),
                Ok(Some(_)) => State::Body(codec.into_payload_codec()),
                Ok(None) => return Some(State::Head(codec)),
                Err(_) => return None,
            },
            State::Body(mut codec) => match codec.decode(buf) {
                Ok(Some(Some(_))) => State::Body(codec),
                Ok(Some(None)) => State::Head(codec.into_message_codec()),
                Ok(None) => return Some(State::Body(codec)),
                Err(_) => return None,
            },
        };
    }
}

// First byte picks how the input is split into reads, to exercise
// partial-buffer parsing the way a real socket delivers data.
fuzz_target!(|data: &[u8]| {
    let Some((&step, rest)) = data.split_first() else {
        return;
    };
    let step = (step as usize % 64) + 1;
    let mut state = CONFIG.with(|c| State::Head(h1::ClientCodec::new(c.clone())));
    let mut buf = BytesMut::new();
    for chunk in rest.chunks(step) {
        buf.extend_from_slice(chunk);
        match drive(state, &mut buf) {
            Some(next) => state = next,
            None => return,
        }
    }
    match state {
        State::Head(mut codec) => {
            let _ = codec.decode_eof(&mut buf);
        }
        State::Body(mut codec) => {
            let _ = codec.decode_eof(&mut buf);
        }
    }
});
