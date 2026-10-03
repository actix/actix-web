use std::task::Context;

use actix_web::{error::PayloadError, web::Bytes};
use futures_core::Stream;
use futures_test::stream::StreamTestExt as _;
use futures_util::{stream, task::noop_waker_ref};
use quickcheck_macros::quickcheck;

use super::Inner;
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

fn run_boundary_parser<S>(
    stream: S,
    boundary: &str,
    buffer_limit: usize,
    parser: fn(&mut PayloadBuffer, &str) -> Result<Option<bool>, crate::error::Error>,
) where
    S: Stream<Item = Result<Bytes, PayloadError>> + 'static,
{
    let mut payload = PayloadBuffer::new_with_limit(stream, buffer_limit);
    let waker = noop_waker_ref();
    let mut cx = Context::from_waker(waker);

    for _ in 0..MAX_QUICKCHECK_POLLS {
        if payload.poll_stream(&mut cx).is_err() {
            return;
        }

        match parser(&mut payload, boundary) {
            Ok(None) => {}
            Ok(Some(_)) | Err(_) => return,
        }
    }
}

fn exercise_boundary_parser(
    chunks: Vec<Vec<u8>>,
    boundary: &str,
    buffer_limit: usize,
    error_seed: u8,
    parser: fn(&mut PayloadBuffer, &str) -> Result<Option<bool>, crate::error::Error>,
) {
    run_boundary_parser(ready_stream(chunks.clone()), boundary, buffer_limit, parser);
    run_boundary_parser(
        adverse_stream(chunks, error_seed),
        boundary,
        buffer_limit,
        parser,
    );
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

fn boundary_line(boundary: &str, final_boundary: bool) -> Vec<u8> {
    let suffix: &[u8] = if final_boundary { b"--\r\n" } else { b"\r\n" };
    let mut line = Vec::with_capacity(boundary.len() + suffix.len() + 2);
    line.extend_from_slice(b"--");
    line.extend_from_slice(boundary.as_bytes());
    line.extend_from_slice(suffix);
    line
}

#[quickcheck]
fn multipart_boundary_parsers_do_not_panic_for_arbitrary_input(
    raw_data: Vec<u8>,
    boundary_seed: Vec<u8>,
    utf8_boundary: String,
    chunk_seed: Vec<u8>,
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
        let valid_limit = boundary.len() + 6 + usize::from(buffer_limit % 32);
        let mut cases = vec![
            data.clone(),
            boundary_line(boundary, false),
            boundary_line(boundary, true),
        ];
        let mut preamble = data.clone();
        preamble.extend_from_slice(b"\r\n");
        preamble.extend_from_slice(&boundary_line(boundary, true));
        cases.push(preamble);

        for case in cases {
            let chunks = fragment(&case, &chunk_seed);
            exercise_boundary_parser(
                chunks.clone(),
                boundary,
                small_limit,
                error_seed,
                Inner::read_boundary,
            );
            exercise_boundary_parser(
                chunks.clone(),
                boundary,
                valid_limit,
                error_seed,
                Inner::read_boundary,
            );
            exercise_boundary_parser(
                chunks.clone(),
                boundary,
                small_limit,
                error_seed,
                Inner::skip_until_boundary,
            );
            exercise_boundary_parser(
                chunks,
                boundary,
                valid_limit,
                error_seed,
                Inner::skip_until_boundary,
            );
        }
    }

    let chunks = fragment(&data, &chunk_seed);
    let empty_boundary_limit = 6 + usize::from(buffer_limit % 32);
    exercise_boundary_parser(
        chunks.clone(),
        "",
        empty_boundary_limit,
        error_seed,
        Inner::read_boundary,
    );
    exercise_boundary_parser(
        chunks,
        "",
        empty_boundary_limit,
        error_seed,
        Inner::skip_until_boundary,
    );
}
