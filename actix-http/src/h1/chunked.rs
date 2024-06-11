use std::{io, task::Poll};

use bytes::{Buf as _, Bytes, BytesMut};
use tracing::{debug, trace};

macro_rules! byte (
    ($rdr:ident) => ({
        if $rdr.len() > 0 {
            let b = $rdr[0];
            $rdr.advance(1);
            b
        } else {
            return Poll::Pending
        }
    })
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChunkedState {
    Size,
    SizeLws,
    Extension,
    SizeLf,
    Body,
    BodyCr,
    BodyLf,
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
            Size => ChunkedState::read_size(body, size),
            SizeLws => ChunkedState::read_size_lws(body),
            Extension => ChunkedState::read_extension(body),
            SizeLf => ChunkedState::read_size_lf(body, *size),
            Body => ChunkedState::read_body(body, size, buf),
            BodyCr => ChunkedState::read_body_cr(body),
            BodyLf => ChunkedState::read_body_lf(body),
            EndCr => ChunkedState::read_end_cr(body),
            EndLf => ChunkedState::read_end_lf(body),
            End => Poll::Ready(Ok(ChunkedState::End)),
        }
    }

    fn read_size(rdr: &mut BytesMut, size: &mut u64) -> Poll<Result<ChunkedState, io::Error>> {
        let radix = 16;

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
                    "Invalid chunk size line: Invalid Size",
                )));
            }
        };

        match size.checked_mul(radix) {
            Some(n) => {
                *size = n;
                *size += rem as u64;

                Poll::Ready(Ok(ChunkedState::Size))
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
            // strictly 0x20 (space) should be disallowed but we don't parse quoted strings here
            0x00..=0x08 | 0x0a..=0x1f | 0x7f => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid character in chunk extension",
            ))),
            _ => Poll::Ready(Ok(ChunkedState::Extension)), // no supported extensions
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
            b'\n' => Poll::Ready(Ok(ChunkedState::Size)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk body LF",
            ))),
        }
    }
    fn read_end_cr(rdr: &mut BytesMut) -> Poll<Result<ChunkedState, io::Error>> {
        match byte!(rdr) {
            b'\r' => Poll::Ready(Ok(ChunkedState::EndLf)),
            _ => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid chunk end CR",
            ))),
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
    use actix_codec::Decoder as _;
    use bytes::{Bytes, BytesMut};
    use http::Method;

    use crate::{
        error::ParseError,
        h1::decoder::{MessageDecoder, PayloadItem},
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

    #[test]
    fn test_parse_chunked_payload_chunk_extension() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
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
             transfer-encoding: chnked\r\n\r\n",
        );
        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_chunked_payload() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
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
             transfer-encoding: chunked\r\n\r\n",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let (req, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();
        assert!(req.chunked().unwrap());

        buf.extend(
            b"4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n\
              POST /test2 HTTP/1.1\r\n\
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
}
