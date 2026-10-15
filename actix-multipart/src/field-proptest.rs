use std::task::{Context, Poll};

use actix_web::{error::PayloadError, web::Bytes};
use futures_core::Stream;
use futures_test::stream::StreamTestExt as _;
use futures_util::{stream, task::noop_waker_ref};
use quickcheck_macros::quickcheck;

use super::InnerField;
use crate::payload::PayloadBuffer;

const MAX_QUICKCHECK_BYTES: usize = 256;
const MAX_QUICKCHECK_BOUNDARY_CHARS: usize = 1024;
const MAX_QUICKCHECK_POLLS: usize = 4096;

fn ready_stream(chunks: Vec<Vec<u8>>) -> impl Stream<Item = Result<Bytes, PayloadError>> {
    stream::iter(
        chunks
            .into_iter()
            .map(|chunk| Ok::<_, PayloadError>(Bytes::from(chunk))),
    )
}

fn adverse_stream(
    chunks: Vec<Vec<u8>>,
    error_seed: u8,
) -> impl Stream<Item = Result<Bytes, PayloadError>> {
    let mut items = Vec::with_capacity(chunks.len() * 2 + 1);

    for chunk in chunks {
        items.push(Ok(Bytes::new()));
        items.push(Ok(Bytes::from(chunk)));
    }

    if items.is_empty() {
        items.push(Ok(Bytes::new()));
    }

    let error_index = usize::from(error_seed) % (items.len() + 1);
    items.insert(error_index, Err(PayloadError::Incomplete(None)));

    stream::iter(items).interleave_pending()
}

fn read_stream_for_test<S>(stream: S, boundary: &str, buffer_limit: usize) -> Result<Vec<u8>, ()>
where
    S: Stream<Item = Result<Bytes, PayloadError>> + 'static,
{
    let mut payload = PayloadBuffer::new_with_limit(stream, buffer_limit);
    let waker = noop_waker_ref();
    let mut cx = Context::from_waker(waker);
    let mut output = Vec::new();

    for _ in 0..MAX_QUICKCHECK_POLLS {
        payload.poll_stream(&mut cx).map_err(|_| ())?;

        match InnerField::read_stream(&mut payload, boundary) {
            Poll::Ready(Some(Ok(chunk))) => output.extend_from_slice(&chunk),
            Poll::Ready(Some(Err(_))) => return Err(()),
            Poll::Ready(None) => return Ok(output),
            Poll::Pending => {}
        }
    }

    Err(())
}

fn read_len_for_test<S>(stream: S, length: u64, buffer_limit: usize) -> Result<Vec<u8>, ()>
where
    S: Stream<Item = Result<Bytes, PayloadError>> + 'static,
{
    let mut payload = PayloadBuffer::new_with_limit(stream, buffer_limit);
    let waker = noop_waker_ref();
    let mut cx = Context::from_waker(waker);
    let mut remaining = length;
    let mut output = Vec::new();

    for _ in 0..MAX_QUICKCHECK_POLLS {
        payload.poll_stream(&mut cx).map_err(|_| ())?;

        match InnerField::read_len(&mut payload, &mut remaining) {
            Poll::Ready(Some(Ok(chunk))) => output.extend_from_slice(&chunk),
            Poll::Ready(Some(Err(_))) => return Err(()),
            Poll::Ready(None) => return Ok(output),
            Poll::Pending => {}
        }
    }

    Err(())
}

fn exercise_read_stream(chunks: Vec<Vec<u8>>, boundary: &str, buffer_limit: usize, error_seed: u8) {
    let _ = read_stream_for_test(ready_stream(chunks.clone()), boundary, buffer_limit);
    let _ = read_stream_for_test(adverse_stream(chunks, error_seed), boundary, buffer_limit);
}

fn exercise_read_len(chunks: Vec<Vec<u8>>, length: u64, buffer_limit: usize, error_seed: u8) {
    let _ = read_len_for_test(ready_stream(chunks.clone()), length, buffer_limit);
    let _ = read_len_for_test(adverse_stream(chunks, error_seed), length, buffer_limit);
}

fn boundary_from(seed: &[u8]) -> String {
    const ALPHABET: &[u8] =
        b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ'()+_,-./:=?";

    let len = seed.first().map_or(1, |byte| 1 + usize::from(*byte) % 70);

    (0..len)
        .map(|index| {
            let byte = seed
                .get(index % seed.len().max(1))
                .copied()
                .unwrap_or_default();
            ALPHABET[usize::from(byte) % ALPHABET.len()] as char
        })
        .collect()
}

fn bounded_utf8_boundary(boundary: String) -> String {
    boundary
        .chars()
        .take(MAX_QUICKCHECK_BOUNDARY_CHARS)
        .collect::<String>()
}

fn fragment(data: &[u8], seed: &[u8]) -> Vec<Vec<u8>> {
    if data.is_empty() {
        return Vec::new();
    }

    if seed.is_empty() {
        return vec![data.to_vec()];
    }

    let mut chunks = Vec::new();
    let mut offset = 0;

    for byte in seed {
        if offset == data.len() {
            break;
        }

        let size = 1 + usize::from(*byte) % 32;
        let end = (offset + size).min(data.len());
        chunks.push(data[offset..end].to_vec());
        offset = end;
    }

    if offset < data.len() {
        chunks.push(data[offset..].to_vec());
    }

    chunks
}

fn field_body(data: &[u8], boundary: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(data.len() + boundary.len() + 6);
    body.extend_from_slice(data);
    body.extend_from_slice(b"\r\n--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"\r\n");
    body
}

#[quickcheck]
fn field_readers_do_not_panic_for_arbitrary_input(
    raw_data: Vec<u8>,
    boundary_seed: Vec<u8>,
    utf8_boundary: String,
    chunk_seed: Vec<u8>,
    length: u8,
    buffer_limit: u8,
) {
    let data = raw_data
        .into_iter()
        .take(MAX_QUICKCHECK_BYTES)
        .collect::<Vec<_>>();
    let ascii_boundary = boundary_from(&boundary_seed);
    let utf8_boundary = bounded_utf8_boundary(utf8_boundary);
    let small_limit = usize::from(buffer_limit % 64);
    let error_seed = chunk_seed.first().copied().unwrap_or_default();

    for boundary in [&ascii_boundary, &utf8_boundary] {
        let body = field_body(&data, boundary);
        let valid_limit = boundary.len() + 6 + usize::from(buffer_limit % 32);

        exercise_read_stream(vec![body.clone()], boundary, valid_limit, error_seed);
        exercise_read_stream(
            fragment(&body, &chunk_seed),
            boundary,
            small_limit,
            error_seed,
        );
        exercise_read_stream(
            fragment(&data, &chunk_seed),
            boundary,
            small_limit,
            error_seed,
        );
    }

    exercise_read_stream(
        fragment(&data, &chunk_seed),
        "",
        6 + usize::from(buffer_limit % 32),
        error_seed,
    );
    exercise_read_len(
        fragment(&data, &chunk_seed),
        u64::from(length),
        small_limit,
        error_seed,
    );
    exercise_read_len(
        vec![data.clone()],
        u64::MAX,
        data.len().max(1) + usize::from(buffer_limit % 32),
        error_seed,
    );
}
