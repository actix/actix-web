use std::{io, task::Poll};

use bytes::{Buf as _, Bytes, BytesMut};
use tracing::{debug, trace};

macro_rules! byte (
    ($rdr:ident) => ({
        match &$rdr[..] {
            &[b, ..] => {
                $rdr.advance(1);
                b
            }
            [] => return Poll::Pending,
        }
    })
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChunkedState {
    SizeFirst,
    SizeRest,
    SizeLws,
    Extension,
    ExtensionQuoted,
    ExtensionQuotedPair,
    SizeLf,
    Body,
    BodyCr,
    BodyLf,
    Trailer,
    TrailerLf,
    EndCr,
    EndLf,
    End,
}

impl ChunkedState {
    pub(super) fn step(
        &self,
        body: &mut BytesMut,
        size: &mut u64,
        buf: &mut Option<Bytes>,
    ) -> Poll<Result<ChunkedState, io::Error>> {
        use self::ChunkedState::*;

        match *self {
            SizeFirst => ChunkedState::read_size_first(body, size),
            SizeRest => ChunkedState::read_size_rest(body, size),
            SizeLws => ChunkedState::read_size_lws(body),
            Extension => ChunkedState::read_extension(body),
            ExtensionQuoted => ChunkedState::read_extension_quoted(body),
            ExtensionQuotedPair => ChunkedState::read_extension_quoted_pair(body),
            SizeLf => ChunkedState::read_size_lf(body, *size),
            Body => ChunkedState::read_body(body, size, buf),
            BodyCr => ChunkedState::read_body_cr(body),
            BodyLf => ChunkedState::read_body_lf(body),
            Trailer => ChunkedState::read_trailer(body),
            TrailerLf => ChunkedState::read_trailer_lf(body),
            EndCr => ChunkedState::read_end_cr(body),
            EndLf => ChunkedState::read_end_lf(body),
            End => Poll::Ready(Ok(ChunkedState::End)),
        }
    }

    fn read_size_first(
        rdr: &mut BytesMut,
        size: &mut u64,
    ) -> Poll<Result<ChunkedState, io::Error>> {
        let rem = match byte!(rdr) {
            b @ b'0'..=b'9' => b - b'0',
            b @ b'a'..=b'f' => b + 10 - b'a',
            b @ b'A'..=b'F' => b + 10 - b'A',
            _ => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid chunk size line: Invalid size first-byte",
                )))
            }
        };

        *size = rem as u64;

        Poll::Ready(Ok(ChunkedState::SizeRest))
    }

    fn read_size_rest(rdr: &mut BytesMut, size: &mut u64) -> Poll<Result<ChunkedState, io::Error>> {
        const HEX_RADIX: u64 = 16;

        let rem = match byte!(rdr) {
            b @ b'0'..=b'9' => b - b'0',
            b @ b'a'..=b'f' => b + 10 - b'a',
            b @ b'A'..=b'F' => b + 10 - b'A',
            b'\t' | b' ' => return Poll::Ready(Ok(ChunkedState::SizeLws)),
            b';' => return Poll::Ready(Ok(ChunkedState::Extension)),
            b'\r' => return Poll::Ready(Ok(ChunkedState::SizeLf)),
            _ => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid chunk size line: Invalid size",
                )));
            }
        };

        match size.checked_mul(HEX_RADIX) {
            Some(n) => {
                *size = n;
                *size += rem as u64;

                Poll::Ready(Ok(ChunkedState::SizeRest))
            }
            None => {
                debug!("chunk size would overflow u64");
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid chunk size line: Size is too big",
                )))
            }
        }
    }

    fn read_size_lws(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        match byte!(rdr) {
            // LWS can follow the chunk size, but no more digits can come
            b'\t' | b' ' => Poll::Ready(Ok(ChunkedState::SizeLws)),
            b';' => Poll::Ready(Ok(ChunkedState::Extension)),
            b'\r' => Poll::Ready(Ok(ChunkedState::SizeLf)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk size linear white space",
            ))),
        }
    }

    fn read_extension(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        match byte!(rdr) {
            b'\r' => Poll::Ready(Ok(ChunkedState::SizeLf)),
            b'"' => Poll::Ready(Ok(ChunkedState::ExtensionQuoted)),
            0x00..=0x08 | 0x0a..=0x1f | 0x7f => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid character in chunk extension",
            ))),
            _ => Poll::Ready(Ok(ChunkedState::Extension)), // no supported extensions
        }
    }

    fn read_extension_quoted(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        // RFC 9110 §5.6.4 relevant BNFs:
        // quoted-string = DQUOTE *( qdtext / quoted-pair ) DQUOTE.
        // qdtext        = HTAB / SP / %x21 / %x23-5B / %x5D-7E / obs-text
        // obs-text      = %x80-FF

        match byte!(rdr) {
            // End of quoted-string; go back to outer extension parser.
            b'"' => Poll::Ready(Ok(ChunkedState::Extension)),

            // Next char is escaped.
            b'\\' => Poll::Ready(Ok(ChunkedState::ExtensionQuotedPair)),

            // Content of quoted-string.
            b'\t' // HTAB (0x09)
            | b' '..=b'~' // other visible ASCII
            | 0x80..=0xff // obs-text
            => Poll::Ready(Ok(ChunkedState::ExtensionQuoted)),

            // Anything else is invalid inside a quoted-string.
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid character in chunk extension quoted string",
            ))),
        }
    }

    fn read_extension_quoted_pair(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        // RFC 9110 §5.6.4 relevant BNFs:
        // quoted-pair    = "\" ( HTAB / SP / VCHAR / obs-text )
        // obs-text      = %x80-FF

        match byte!(rdr) {
            // Escape-able byte; go back to quoted-string parser.
            b'\t' // HTAB
            | b' '..=b'~' // visible ASCII
            | 0x80..=0xff // obs-test
            => Poll::Ready(Ok(ChunkedState::ExtensionQuoted)),

            // Anything else is invalid as the second byte of a quoted-pair.
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid character in chunk extension quoted-pair",
            ))),
        }
    }

    fn read_size_lf(rdr: &mut BytesMut, size: u64) -> Poll<Result<ChunkedState, io::Error>> {
        match byte!(rdr) {
            b'\n' if size > 0 => Poll::Ready(Ok(ChunkedState::Body)),
            b'\n' if size == 0 => Poll::Ready(Ok(ChunkedState::EndCr)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk size LF",
            ))),
        }
    }

    fn read_body(
        rdr: &mut BytesMut,
        rem: &mut u64,
        buf: &mut Option<Bytes>,
    ) -> Poll<Result<ChunkedState, io::Error>> {
        trace!("Chunked read, remaining={:?}", rem);

        let len = rdr.len() as u64;
        if len == 0 {
            Poll::Ready(Ok(ChunkedState::Body))
        } else {
            let slice;
            if *rem > len {
                slice = rdr.split().freeze();
                *rem -= len;
            } else {
                slice = rdr.split_to(*rem as usize).freeze();
                *rem = 0;
            }
            *buf = Some(slice);
            if *rem > 0 {
                Poll::Ready(Ok(ChunkedState::Body))
            } else {
                Poll::Ready(Ok(ChunkedState::BodyCr))
            }
        }
    }

    fn read_body_cr(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        match byte!(rdr) {
            b'\r' => Poll::Ready(Ok(ChunkedState::BodyLf)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk body CR",
            ))),
        }
    }
    fn read_body_lf(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        match byte!(rdr) {
            b'\n' => Poll::Ready(Ok(ChunkedState::SizeFirst)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk body LF",
            ))),
        }
    }

    fn read_trailer(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        loop {
            match byte!(rdr) {
                // End of trailer signal. Must read LF next.
                b'\r' => return Poll::Ready(Ok(ChunkedState::TrailerLf)),

                // Trailer field lines must end with CRLF, not a bare LF.
                b'\n' => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid chunk trailer CR",
                    )))
                }

                // Allow horizontal tab, which is a control char but allowed by RFC 9110 §5.5.
                0x09 => continue,

                // Reject other control bytes that cannot appear within a trailer field line.
                b if b.is_ascii_control() => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid character in chunk trailer",
                    )))
                }

                // Continue reading this trailer's bytes.
                _ => continue,
            }
        }
    }

    fn read_trailer_lf(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        match byte!(rdr) {
            b'\n' => Poll::Ready(Ok(ChunkedState::EndCr)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk trailer LF",
            ))),
        }
    }

    fn read_end_cr(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        match byte!(rdr) {
            b'\r' => Poll::Ready(Ok(ChunkedState::EndLf)),
            0x00..=0x08 | 0x0a..=0x1f | 0x7f => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid chunk end CR",
            ))),
            _ => Poll::Ready(Ok(ChunkedState::Trailer)),
        }
    }

    fn read_end_lf(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        match byte!(rdr) {
            b'\n' => Poll::Ready(Ok(ChunkedState::End)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk end LF",
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io, iter};

    use actix_codec::Decoder as _;
    use bytes::{Bytes, BytesMut};
    use http::Method;

    use crate::{
        error::ParseError,
        h1::decoder::{MessageDecoder, PayloadDecoder, PayloadItem},
        HttpMessage as _, Request,
    };

    macro_rules! parse_ready {
        ($e:expr) => {{
            match MessageDecoder::<Request>::default().decode($e) {
                Ok(Some((msg, _))) => msg,
                Ok(_) => unreachable!("Eof during parsing http request"),
                Err(err) => unreachable!("Error during parsing http request: {:?}", err),
            }
        }};
    }

    macro_rules! expect_parse_err {
        ($e:expr) => {{
            match MessageDecoder::<Request>::default().decode($e) {
                Err(err) => match err {
                    ParseError::Io(_) => unreachable!("Parse error expected"),
                    _ => {}
                },
                _ => unreachable!("Error expected"),
            }
        }};
    }

    macro_rules! assert_decode_chunk_eq {
        ($pl:expr, $buf:expr, $expected:expr) => {
            assert_eq!(
                $pl.decode(&mut $buf).unwrap().unwrap().chunk().as_ref(),
                $expected,
            );
        };
    }

    macro_rules! assert_decode_chunked_eof {
        ($pl:expr, $buf:expr) => {
            assert!($pl.decode(&mut $buf).unwrap().unwrap().eof());
        };
    }

    #[test]
    fn test_reject_empty_chunk_size() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             Host: localhost\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let (_req, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();

        // Empty chunk size before CRLF is illegal under RFC 9112 §7.1
        buf.extend(b"\r\n\r\n");
        assert!(pl.decode(&mut buf).is_err());
    }

    #[test]
    fn test_parse_chunked_payload_chunk_extension() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
            Host: localhost\r\n\
            transfer-encoding: chunked\r\n\
            \r\n",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let (msg, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();
        assert!(msg.chunked().unwrap());

        buf.extend(b"4;test\r\ndata\r\n4\r\nline\r\n0\r\n\r\n"); // test: test\r\n\r\n")
        let chunk = pl.decode(&mut buf).unwrap().unwrap().chunk();
        assert_eq!(chunk, Bytes::from_static(b"data"));
        let chunk = pl.decode(&mut buf).unwrap().unwrap().chunk();
        assert_eq!(chunk, Bytes::from_static(b"line"));
        let msg = pl.decode(&mut buf).unwrap().unwrap();
        assert!(msg.eof());
    }

    #[test]
    fn test_request_chunked() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             Host: localhost\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        if let Ok(val) = req.chunked() {
            assert!(val);
        } else {
            unreachable!("Error");
        }

        // intentional typo in "chunked"
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             Host: localhost\r\n\
             transfer-encoding: chnked\r\n\r\n",
        );
        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_chunked_payload() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             Host: localhost\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let (req, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();
        assert!(req.chunked().unwrap());

        buf.extend(b"4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n");
        assert_eq!(
            pl.decode(&mut buf).unwrap().unwrap().chunk().as_ref(),
            b"data"
        );
        assert_eq!(
            pl.decode(&mut buf).unwrap().unwrap().chunk().as_ref(),
            b"line"
        );
        assert!(pl.decode(&mut buf).unwrap().unwrap().eof());
    }

    #[test]
    fn test_http_request_chunked_payload_and_next_message() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             Host: localhost\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let (req, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();
        assert!(req.chunked().unwrap());

        buf.extend(
            b"4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n\
              POST /test2 HTTP/1.1\r\n\
              Host: localhost\r\n\
              transfer-encoding: chunked\r\n\r\n"
                .iter(),
        );
        let msg = pl.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"data");
        let msg = pl.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"line");
        let msg = pl.decode(&mut buf).unwrap().unwrap();
        assert!(msg.eof());

        let (req, _) = reader.decode(&mut buf).unwrap().unwrap();
        assert!(req.chunked().unwrap());
        assert_eq!(*req.method(), Method::POST);
        assert!(req.chunked().unwrap());
    }

    #[test]
    fn test_http_request_chunked_payload_chunks() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             Host: localhost\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let (req, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();
        assert!(req.chunked().unwrap());

        buf.extend(b"4\r\n1111\r\n");
        let msg = pl.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"1111");

        buf.extend(b"4\r\ndata\r");
        let msg = pl.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"data");

        buf.extend(b"\n4");
        assert!(pl.decode(&mut buf).unwrap().is_none());

        buf.extend(b"\r");
        assert!(pl.decode(&mut buf).unwrap().is_none());
        buf.extend(b"\n");
        assert!(pl.decode(&mut buf).unwrap().is_none());

        buf.extend(b"li");
        let msg = pl.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"li");

        //trailers
        //buf.feed_data("test: test\r\n");
        //not_ready!(reader.parse(&mut buf, &mut readbuf));

        buf.extend(b"ne\r\n0\r\n");
        let msg = pl.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"ne");
        assert!(pl.decode(&mut buf).unwrap().is_none());

        buf.extend(b"\r\n");
        assert!(pl.decode(&mut buf).unwrap().unwrap().eof());
    }

    #[test]
    fn chunk_extension_quoted() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
            Host: localhost:8080\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            2;hello=b;one=\"1 2 3\"\r\n\
            xx",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let (_msg, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();

        let chunk = pl.decode(&mut buf).unwrap().unwrap();
        assert_eq!(chunk, PayloadItem::Chunk(Bytes::from_static(b"xx")));
    }

    #[test]
    fn chunk_extension_quoted_text() {
        let mut pl = PayloadDecoder::chunked();
        let mut buf = BytesMut::from(b"1;ext=\"".as_slice());

        // For all valid chars inside a quoted-string.
        for byte in iter::once(b'\t').chain(0x20..=0x7e).chain(0x80..=0xff) {
            // But skip the ones which need escaping.
            if !matches!(byte, b'"' | b'\\') {
                buf.extend_from_slice(&[byte]);
            }
        }

        buf.extend_from_slice(b"\"\r\nx\r\n0\r\n\r\n");

        assert_decode_chunk_eq!(pl, buf, b"x");
        assert_decode_chunked_eof!(pl, buf);
        assert!(buf.is_empty(), "Decoder should consume the chunked payload");
    }

    #[test]
    fn chunk_extension_quoted_pairs() {
        // For all valid chars inside a quoted-string.
        for byte in iter::once(b'\t').chain(0x20..=0x7e).chain(0x80..=0xff) {
            let mut input = BytesMut::from(b"1;ext=\"\\".as_slice());
            input.extend_from_slice(&[byte]);
            input.extend_from_slice(b"\";next=\"end\\\\\"\r\nx\r\n0;ext=\"\\\"\\\\\"\r\n\r\n");

            // Test that one-byte-at-a-time and single-chunk cases yield same parsing result.
            for read_size in [1, input.len()] {
                let mut pl = PayloadDecoder::chunked();
                let mut buf = BytesMut::new();
                let mut body = BytesMut::new();
                let mut eof = false;

                for chunk in input.chunks(read_size) {
                    buf.extend_from_slice(chunk);

                    while let Some(item) = pl
                        .decode(&mut buf)
                        .expect("Valid quoted-pair bytes should decode")
                    {
                        match item {
                            PayloadItem::Chunk(chunk) => body.extend_from_slice(&chunk),
                            PayloadItem::Eof => {
                                eof = true;
                                break;
                            }
                        }
                    }

                    assert!(buf.is_empty(), "Decoder should consume the available input");
                }

                assert_eq!(body.as_ref(), b"x", "Quoted-pairs should preserve the body");
                assert!(
                    eof,
                    "Decoder should reach EOF for quoted-pair byte {byte:#04x} with read size {read_size}"
                );
            }
        }
    }

    #[test]
    fn chunk_extension_quoted_rejects_control_bytes() {
        // For all control chars inside a quoted-string or quoted-pair.
        for byte in (0x00..=0x1f).chain([0x7f]) {
            // But skip HTAB, which is valid in both cases.
            if byte == b'\t' {
                continue;
            }

            // Test each char both without escaping and after an escape backslash.
            for prefix in [b"1;ext=\"".as_slice(), b"1;ext=\"\\"] {
                let mut pl = PayloadDecoder::chunked();
                let mut buf = BytesMut::from(prefix);
                assert!(
                    pl.decode(&mut buf)
                        .expect("An incomplete quoted extension should remain valid")
                        .is_none(),
                    "Decoder should wait for the rest of the quoted extension"
                );
                assert!(
                    buf.is_empty(),
                    "Decoder should consume the extension prefix"
                );

                buf.extend_from_slice(&[byte]);

                // Reject the byte without waiting for more input or connection EOF.
                let err = pl.decode(&mut buf).expect_err(
                    "Control bytes in a quoted extension should be rejected immediately",
                );
                assert_eq!(
                    err.kind(),
                    io::ErrorKind::InvalidData,
                    "Invalid byte {byte:#04x} should produce InvalidData"
                );
            }
        }
    }

    #[test]
    fn chunk_extension_quoted_pair_keeps_next_request_intact() {
        let next_request = b"GET /next HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut remaining = BytesMut::from(b"\nx\r\n0\r\n\r\n".as_slice());
        remaining.extend_from_slice(next_request);

        // An escaped quote cannot close the string, and a backslash cannot escape CR.
        for prefix in [b"1;ext=\"\\\"".as_slice(), b"1;ext=\"\\"] {
            // Test every split point so quote and escape state must survive partial reads.
            for split in 0..=prefix.len() {
                let mut pl = PayloadDecoder::chunked();
                let mut buf = BytesMut::from(&prefix[..split]);
                assert!(
                    pl.decode(&mut buf)
                        .expect("An incomplete quoted extension should remain valid")
                        .is_none(),
                    "Decoder should wait for the rest of the quoted extension"
                );
                assert!(
                    buf.is_empty(),
                    "Decoder should consume the extension prefix"
                );

                // Put the invalid CR, the rest of the body, and the next request in one read.
                buf.extend_from_slice(&prefix[split..]);
                buf.extend_from_slice(b"\r");
                buf.extend_from_slice(&remaining);

                let err = pl
                    .decode(&mut buf)
                    .expect_err("CR inside a quoted extension should be rejected");
                assert_eq!(
                    err.kind(),
                    io::ErrorKind::InvalidData,
                    "CR should produce InvalidData"
                );
                assert_eq!(
                    buf, remaining,
                    "Decoder should leave the bytes after the invalid CR unread"
                );
            }
        }
    }

    #[test]
    fn hrs_chunk_extension_invalid() {
        let mut buf = BytesMut::from(
            "GET / HTTP/1.1\r\n\
            Host: localhost:8080\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            2;x\nx\r\n\
            4c\r\n\
            0\r\n",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let (_msg, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();

        let err = pl.decode(&mut buf).unwrap_err();
        assert!(err
            .to_string()
            .contains("Invalid character in chunk extension"));
    }

    #[test]
    fn hrs_chunk_size_overflow() {
        let mut buf = BytesMut::from(
            "GET / HTTP/1.1\r\n\
            Host: example.com\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            f0000000000000003\r\n\
            abc\r\n\
            0\r\n",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let (_msg, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();

        let err = pl.decode(&mut buf).unwrap_err();
        assert!(err
            .to_string()
            .contains("Invalid chunk size line: Size is too big"));
    }

    #[test]
    fn test_parse_chunked_payload_with_trailers() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             Host: localhost\r\n\
             transfer-encoding: chunked\r\n\r\n\
             4\r\ndata\r\n\
             0\r\n\
             X-Checksum: 12345\r\n\
             Expires: Wed\r\n\r\n",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let mut pl = reader.decode(&mut buf).unwrap().unwrap().1.unwrap();

        assert_decode_chunk_eq!(pl, buf, b"data");
        assert_decode_chunked_eof!(pl, buf);

        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             Host: localhost\r\n\
             transfer-encoding: chunked\r\n\r\n\
             4\r\ndata\r\n\
             0\r\n\
               X-Checksum: 12345\r\n\
             Expires: Wed  \r\n\r\n",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let mut pl = reader.decode(&mut buf).unwrap().unwrap().1.unwrap();

        assert_decode_chunk_eq!(pl, buf, b"data");
        assert_decode_chunked_eof!(pl, buf);
    }

    #[test]
    fn test_chunked_bare_lf_in_trailer_without_follow_up() {
        // Read the last chunk and partial trailer both byte by byte and in one read.
        for chunk_size in [1, 4] {
            let mut pl = PayloadDecoder::chunked();
            let mut buf = BytesMut::new();

            for chunk in b"0\r\nX".chunks(chunk_size) {
                buf.extend_from_slice(chunk);
                assert!(pl.decode(&mut buf).unwrap().is_none());
                assert!(buf.is_empty());
            }

            buf.extend_from_slice(b"\n");

            // Reject LF immediately, without waiting for a later CR or connection EOF.
            let err = pl.decode(&mut buf).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert!(buf.is_empty());
        }
    }

    #[test]
    fn test_chunked_trailer_rejects_control_bytes() {
        for byte in (0x00..=0x1f).chain([0x7f]) {
            // HT is valid field whitespace, and CR starts the line terminator.
            if matches!(byte, b'\t' | b'\r') {
                continue;
            }

            // Check both the first trailer byte and a byte inside the field value.
            for prefix in [b"0\r\n".as_slice(), b"0\r\nX: "] {
                let mut pl = PayloadDecoder::chunked();
                let mut buf = BytesMut::from(prefix);
                buf.extend_from_slice(&[byte]);

                let err = pl.decode(&mut buf).unwrap_err();
                assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            }
        }
    }

    #[test]
    fn test_chunked_trailers_partial_reads() {
        let mut pl = PayloadDecoder::chunked();
        let mut buf = BytesMut::new();

        // Split both trailer lines and each CRLF across reads. HT and obs-text remain valid.
        for byte in b"0\r\nX: 1\t\x80\xff\r\nY: 2\r\n\r" {
            buf.extend_from_slice(&[*byte]);
            assert!(pl.decode(&mut buf).unwrap().is_none());
            assert!(buf.is_empty());
        }

        // Only the final LF completes the empty line after the trailers.
        buf.extend_from_slice(b"\n");
        assert_decode_chunked_eof!(pl, buf);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_chunked_bare_lf_in_trailer_keeps_next_request_intact() {
        let mut buf = BytesMut::from(
            "POST /first HTTP/1.1\r\n\
             Host: localhost\r\n\
             Transfer-Encoding: chunked\r\n\r\n",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let mut pl = reader.decode(&mut buf).unwrap().unwrap().1.unwrap();

        let next_request = b"POST /second HTTP/1.1\r\n\
                             Host: localhost\r\n\
                             Content-Length: 2\r\n\r\nab";

        // End the trailer with bare LF, then append a complete request in the same buffer.
        buf.extend_from_slice(b"5\r\nhello\r\n0\r\nX\n");
        buf.extend_from_slice(next_request);

        assert_decode_chunk_eq!(pl, buf, b"hello");

        // Stop at the invalid LF and leave the entire next request, including its body, unread.
        let err = pl.decode(&mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(buf.as_ref(), next_request);
    }

    #[test]
    fn test_chunked_bare_lf_after_last_chunk_keeps_next_request_intact() {
        let mut buf = BytesMut::from(
            "POST /test HTTP/1.1\r\n\
             Host: localhost\r\n\
             transfer-encoding: chunked\r\n\
             \r\n",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let (req, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();
        assert!(req.chunked().unwrap());

        // The last chunk is terminated by a bare LF instead of CRLF and a second
        // request is pipelined onto the same connection.
        buf.extend(
            b"5\r\nhello\r\n0\r\n\n\
              POST /front HTTP/1.1\r\n\
              Host: localhost\r\n\
              Content-Length: 2\r\n\r\n\
              ab"
            .iter(),
        );

        let msg = pl.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"hello");

        // Reject the bare LF without consuming the pipelined request.
        let err = pl.decode(&mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        assert!(
            buf.as_ref().starts_with(b"POST /front"),
            "Pipelined request head was consumed: {:?}",
            String::from_utf8_lossy(&buf)
        );
    }
}
