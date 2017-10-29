use std::{self, io, ptr};

use httparse;
use http::{Method, Version, HttpTryFrom, HeaderMap};
use http::header::{self, HeaderName, HeaderValue};
use bytes::{BytesMut, BufMut};
use futures::{Async, Poll};
use tokio_io::AsyncRead;
use percent_encoding;

use error::ParseError;
use decode::Decoder;
use httprequest::HttpRequest;
use payload::{Payload, PayloadError, PayloadSender};

const MAX_HEADERS: usize = 100;
const INIT_BUFFER_SIZE: usize = 8192;
const MAX_BUFFER_SIZE: usize = 131_072;

enum Decoding {
    Paused,
    Ready,
    NotReady,
}

struct PayloadInfo {
    tx: PayloadSender,
    decoder: Decoder,
}

pub(crate) struct Reader {
    read_buf: BytesMut,
    payload: Option<PayloadInfo>,
}

#[derive(Debug)]
pub(crate) enum ReaderError {
    Payload,
    Error(ParseError),
}

impl Reader {
    pub fn new() -> Reader {
        Reader {
            read_buf: BytesMut::new(),
            payload: None,
        }
    }

    fn decode(&mut self) -> std::result::Result<Decoding, ReaderError>
    {
        if let Some(ref mut payload) = self.payload {
            if payload.tx.maybe_paused() {
                return Ok(Decoding::Paused)
            }
            loop {
                match payload.decoder.decode(&mut self.read_buf) {
                    Ok(Async::Ready(Some(bytes))) => {
                        payload.tx.feed_data(bytes)
                    },
                    Ok(Async::Ready(None)) => {
                        payload.tx.feed_eof();
                        return Ok(Decoding::Ready)
                    },
                    Ok(Async::NotReady) => return Ok(Decoding::NotReady),
                    Err(err) => {
                        payload.tx.set_error(err.into());
                        return Err(ReaderError::Payload)
                    }
                }
            }
        } else {
            return Ok(Decoding::Ready)
        }
    }
    
    pub fn parse<T>(&mut self, io: &mut T) -> Poll<(HttpRequest, Payload), ReaderError>
        where T: AsyncRead
    {
        loop {
            match self.decode()? {
                Decoding::Paused => return Ok(Async::NotReady),
                Decoding::Ready => {
                    self.payload = None;
                    break
                },
                Decoding::NotReady => {
                    match self.read_from_io(io) {
                        Ok(Async::Ready(0)) => {
                            if let Some(ref mut payload) = self.payload {
                                payload.tx.set_error(PayloadError::Incomplete);
                            }
                            // http channel should not deal with payload errors
                            return Err(ReaderError::Payload)
                        }
                        Ok(Async::Ready(_)) => {
                            continue
                        }
                        Ok(Async::NotReady) => break,
                        Err(err) => {
                            if let Some(ref mut payload) = self.payload {
                                payload.tx.set_error(err.into());
                            }
                            // http channel should not deal with payload errors
                            return Err(ReaderError::Payload)
                        }
                    }
                }
            }
        }

        loop {
            match try!(Reader::parse_message(&mut self.read_buf).map_err(ReaderError::Error)) {
                Some((msg, decoder)) => {
                    let payload = if let Some(decoder) = decoder {
                        let (tx, rx) = Payload::new(false);
                        let payload = PayloadInfo {
                            tx: tx,
                            decoder: decoder,
                        };
                        self.payload = Some(payload);

                        loop {
                            match self.decode()? {
                                Decoding::Paused =>
                                    break,
                                Decoding::Ready => {
                                    self.payload = None;
                                    break
                                },
                                Decoding::NotReady => {
                                    match self.read_from_io(io) {
                                        Ok(Async::Ready(0)) => {
                                            trace!("parse eof");
                                            if let Some(ref mut payload) = self.payload {
                                                payload.tx.set_error(PayloadError::Incomplete);
                                            }
                                            // http channel should deal with payload errors
                                            return Err(ReaderError::Payload)
                                        }
                                        Ok(Async::Ready(_)) => {
                                            continue
                                        }
                                        Ok(Async::NotReady) => break,
                                        Err(err) => {
                                            if let Some(ref mut payload) = self.payload {
                                                payload.tx.set_error(err.into());
                                            }
                                            // http channel should deal with payload errors
                                            return Err(ReaderError::Payload)
                                        }
                                    }
                                }
                            }
                        }
                        rx
                    } else {
                        let (_, rx) = Payload::new(true);
                        rx
                    };
                    return Ok(Async::Ready((msg, payload)));
                },
                None => {
                    if self.read_buf.capacity() >= MAX_BUFFER_SIZE {
                        debug!("MAX_BUFFER_SIZE reached, closing");
                        return Err(ReaderError::Error(ParseError::TooLarge));
                    }
                },
            }
            match self.read_from_io(io) {
                Ok(Async::Ready(0)) => {
                    trace!("Eof during parse");
                    return Err(ReaderError::Error(ParseError::Incomplete));
                },
                Ok(Async::Ready(_)) => (),
                Ok(Async::NotReady) =>
                    return Ok(Async::NotReady),
                Err(err) =>
                    return Err(ReaderError::Error(err.into()))
            }
        }
    }

    fn read_from_io<T: AsyncRead>(&mut self, io: &mut T) -> Poll<usize, io::Error> {
        if self.read_buf.remaining_mut() < INIT_BUFFER_SIZE {
            self.read_buf.reserve(INIT_BUFFER_SIZE);
            unsafe { // Zero out unused memory
                let buf = self.read_buf.bytes_mut();
                let len = buf.len();
                ptr::write_bytes(buf.as_mut_ptr(), 0, len);
            }
        }
        unsafe {
            let n = match io.read(self.read_buf.bytes_mut()) {
                Ok(n) => n,
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        return Ok(Async::NotReady);
                    }
                    return Err(e)
                }
            };
            self.read_buf.advance_mut(n);
            Ok(Async::Ready(n))
        }
    }

    fn parse_message(buf: &mut BytesMut)
                     -> Result<Option<(HttpRequest, Option<Decoder>)>, ParseError>
    {
        if buf.is_empty() {
            return Ok(None);
        }

        // Parse http message
        let mut headers_indices = [HeaderIndices {
            name: (0, 0),
            value: (0, 0)
        }; MAX_HEADERS];

        let (len, method, path, version, headers_len) = {
            let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
            trace!("Request.parse([Header; {}], [u8; {}])", headers.len(), buf.len());
            let mut req = httparse::Request::new(&mut headers);
            match try!(req.parse(buf)) {
                httparse::Status::Complete(len) => {
                    trace!("Request.parse Complete({})", len);
                    let method = Method::try_from(req.method.unwrap())
                        .map_err(|_| ParseError::Method)?;
                    let path = req.path.unwrap();
                    let bytes_ptr = buf.as_ref().as_ptr() as usize;
                    let path_start = path.as_ptr() as usize - bytes_ptr;
                    let path_end = path_start + path.len();
                    let path = (path_start, path_end);

                    let version = if req.version.unwrap() == 1 {
                        Version::HTTP_11
                    } else {
                        Version::HTTP_10
                    };

                    record_header_indices(buf.as_ref(), req.headers, &mut headers_indices);
                    let headers_len = req.headers.len();
                    (len, method, path, version, headers_len)
                }
                httparse::Status::Partial => return Ok(None),
            }
        };

        let slice = buf.split_to(len).freeze();
        let path = slice.slice(path.0, path.1);

        // manually split path, path was found to be utf8 by httparse
        let uri = {
            if let Ok(path) = percent_encoding::percent_decode(&path).decode_utf8() {
                let parts: Vec<&str> = path.splitn(2, '?').collect();
                if parts.len() == 2 {
                    Some((parts[0].to_owned(), parts[1].to_owned()))
                } else {
                    Some((parts[0].to_owned(), String::new()))
                }
            } else {
                None
            }
        };
        let (path, query) = if let Some(uri) = uri {
            uri
        } else {
            let parts: Vec<&str> = unsafe{
                std::str::from_utf8_unchecked(&path)}.splitn(2, '?').collect();
            if parts.len() == 2 {
                (parts[0].to_owned(), parts[1][1..].to_owned())
            } else {
                (parts[0].to_owned(), String::new())
            }
        };

        // convert headers
        let mut headers = HeaderMap::with_capacity(headers_len);
        for header in headers_indices[..headers_len].iter() {
            if let Ok(name) = HeaderName::try_from(slice.slice(header.name.0, header.name.1)) {
                if let Ok(value) = HeaderValue::try_from(
                    slice.slice(header.value.0, header.value.1))
                {
                    headers.append(name, value);
                } else {
                    return Err(ParseError::Header)
                }
            } else {
                return Err(ParseError::Header)
            }
        }

        let msg = HttpRequest::new(method, path, version, headers, query);

        let decoder = if msg.upgrade() {
            Some(Decoder::eof())
        } else {
            let chunked = msg.chunked()?;

            // Content-Length
            if let Some(len) = msg.headers().get(header::CONTENT_LENGTH) {
                if chunked {
                    return Err(ParseError::Header)
                }
                if let Ok(s) = len.to_str() {
                    if let Ok(len) = s.parse::<u64>() {
                        Some(Decoder::length(len))
                    } else {
                        debug!("illegal Content-Length: {:?}", len);
                        return Err(ParseError::Header)
                    }
                } else {
                    debug!("illegal Content-Length: {:?}", len);
                    return Err(ParseError::Header)
                }
            } else if chunked {
                Some(Decoder::chunked())
            } else {
                None
            }
        };
        Ok(Some((msg, decoder)))
    }
}

#[derive(Clone, Copy)]
struct HeaderIndices {
    name: (usize, usize),
    value: (usize, usize),
}

fn record_header_indices(bytes: &[u8],
                         headers: &[httparse::Header],
                         indices: &mut [HeaderIndices])
{
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


#[cfg(test)]
mod tests {
    use std::{io, cmp};
    use bytes::{Bytes, BytesMut};
    use futures::{Async};
    use tokio_io::AsyncRead;
    use http::{Version, Method};
    use super::{Reader, ReaderError};

    struct Buffer {
        buf: Bytes,
        err: Option<io::Error>,
    }

    impl Buffer {
        fn new(data: &'static str) -> Buffer {
            Buffer {
                buf: Bytes::from(data),
                err: None,
            }
        }
        fn feed_data(&mut self, data: &'static str) {
            let mut b = BytesMut::from(self.buf.as_ref());
            b.extend(data.as_bytes());
            self.buf = b.take().freeze();
        }
    }
    
    impl AsyncRead for Buffer {}
    impl io::Read for Buffer {
        fn read(&mut self, dst: &mut [u8]) -> Result<usize, io::Error> {
            if self.buf.is_empty() {
                if self.err.is_some() {
                    Err(self.err.take().unwrap())
                } else {
                    Err(io::Error::new(io::ErrorKind::WouldBlock, ""))
                }
            } else {
                let size = cmp::min(self.buf.len(), dst.len());
                let b = self.buf.split_to(size);
                dst[..size].copy_from_slice(&b);
                Ok(size)
            }
        }
    }

    macro_rules! not_ready {
        ($e:expr) => (match $e {
            Ok(Async::NotReady) => (),
            Err(err) => panic!("Unexpected error: {:?}", err),
            _ => panic!("Should not be ready"),
        })
    }

    macro_rules! parse_ready {
        ($e:expr) => (
            match Reader::new().parse($e) {
                Ok(Async::Ready((req, payload))) => (req, payload),
                Ok(_) => panic!("Eof during parsing http request"),
                Err(err) => panic!("Error during parsing http request: {:?}", err),
            }
        )
    }

    macro_rules! reader_parse_ready {
        ($e:expr) => (
            match $e {
                Ok(Async::Ready((req, payload))) => (req, payload),
                Ok(_) => panic!("Eof during parsing http request"),
                Err(err) => panic!("Error during parsing http request: {:?}", err),
            }
        )
    }

    macro_rules! expect_parse_err {
        ($e:expr) => (match Reader::new().parse($e) {
            Err(err) => match err {
                ReaderError::Error(_) => (),
                _ => panic!("Parse error expected"),
            },
            _ => panic!("Error expected"),
        })
    }

    #[test]
    fn test_parse() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n\r\n");

        let mut reader = Reader::new();
        match reader.parse(&mut buf) {
            Ok(Async::Ready((req, payload))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert!(payload.eof());
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_partial() {
        let mut buf = Buffer::new("PUT /test HTTP/1");

        let mut reader = Reader::new();
        match reader.parse(&mut buf) {
            Ok(Async::NotReady) => (),
            _ => panic!("Error"),
        }

        buf.feed_data(".1\r\n\r\n");
        match reader.parse(&mut buf) {
            Ok(Async::Ready((req, payload))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::PUT);
                assert_eq!(req.path(), "/test");
                assert!(payload.eof());
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_post() {
        let mut buf = Buffer::new("POST /test2 HTTP/1.0\r\n\r\n");

        let mut reader = Reader::new();
        match reader.parse(&mut buf) {
            Ok(Async::Ready((req, payload))) => {
                assert_eq!(req.version(), Version::HTTP_10);
                assert_eq!(*req.method(), Method::POST);
                assert_eq!(req.path(), "/test2");
                assert!(payload.eof());
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_body() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");

        let mut reader = Reader::new();
        match reader.parse(&mut buf) {
            Ok(Async::Ready((req, mut payload))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(payload.readall().unwrap().as_ref(), b"body");
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_body_crlf() {
        let mut buf = Buffer::new(
            "\r\nGET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");

        let mut reader = Reader::new();
        match reader.parse(&mut buf) {
            Ok(Async::Ready((req, mut payload))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(payload.readall().unwrap().as_ref(), b"body");
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_partial_eof() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n");

        let mut reader = Reader::new();
        not_ready!{ reader.parse(&mut buf) }

        buf.feed_data("\r\n");
        match reader.parse(&mut buf) {
            Ok(Async::Ready((req, payload))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert!(payload.eof());
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_headers_split_field() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n");

        let mut reader = Reader::new();
        not_ready!{ reader.parse(&mut buf) }

        buf.feed_data("t");
        not_ready!{ reader.parse(&mut buf) }

        buf.feed_data("es");
        not_ready!{ reader.parse(&mut buf) }

        buf.feed_data("t: value\r\n\r\n");
        match reader.parse(&mut buf) {
            Ok(Async::Ready((req, payload))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(req.headers().get("test").unwrap().as_bytes(), b"value");
                assert!(payload.eof());
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_headers_multi_value() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             Set-Cookie: c1=cookie1\r\n\
             Set-Cookie: c2=cookie2\r\n\r\n");

        let mut reader = Reader::new();
        match reader.parse(&mut buf) {
            Ok(Async::Ready((req, _))) => {
                let val: Vec<_> = req.headers().get_all("Set-Cookie")
                    .iter().map(|v| v.to_str().unwrap().to_owned()).collect();
                assert_eq!(val[0], "c1=cookie1");
                assert_eq!(val[1], "c2=cookie2");
            }
            Ok(_) | Err(_) => panic!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_conn_default_1_0() {
        let mut buf = Buffer::new("GET /test HTTP/1.0\r\n\r\n");
        let (req, _) = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_default_1_1() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n\r\n");
        let (req, _) = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_close() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: close\r\n\r\n");
        let (req, _) = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_close_1_0() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.0\r\n\
             connection: close\r\n\r\n");
        let (req, _) = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_keep_alive_1_0() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.0\r\n\
             connection: keep-alive\r\n\r\n");
        let (req, _) = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_keep_alive_1_1() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: keep-alive\r\n\r\n");
        let (req, _) = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_other_1_0() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.0\r\n\
             connection: other\r\n\r\n");
        let (req, _) = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_other_1_1() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: other\r\n\r\n");
        let (req, _) = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_upgrade() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: upgrade\r\n\r\n");
        let (req, payload) = parse_ready!(&mut buf);

        assert!(!payload.eof());
        assert!(req.upgrade());
    }

    #[test]
    fn test_conn_upgrade_connect_method() {
        let mut buf = Buffer::new(
            "CONNECT /test HTTP/1.1\r\n\
             content-length: 0\r\n\r\n");
        let (req, payload) = parse_ready!(&mut buf);

        assert!(req.upgrade());
        assert!(!payload.eof());
    }

    #[test]
    fn test_request_chunked() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");
        let (req, payload) = parse_ready!(&mut buf);

        assert!(!payload.eof());
        if let Ok(val) = req.chunked() {
            assert!(val);
        } else {
            panic!("Error");
        }

        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chnked\r\n\r\n");
        let (req, payload) = parse_ready!(&mut buf);

        assert!(payload.eof());
        if let Ok(val) = req.chunked() {
            assert!(!val);
        } else {
            panic!("Error");
        }
    }

    #[test]
    fn test_headers_content_length_err_1() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             content-length: line\r\n\r\n");

        expect_parse_err!(&mut buf)
    }

    #[test]
    fn test_headers_content_length_err_2() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             content-length: -1\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_invalid_header() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             test line\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_invalid_name() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             test[]: line\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_bad_status_line() {
        let mut buf = Buffer::new("getpath \r\n\r\n");
        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_upgrade() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: upgrade\r\n\
             upgrade: websocket\r\n\r\n\
             some raw data");
        let (req, mut payload) = parse_ready!(&mut buf);
        assert!(!req.keep_alive());
        assert!(req.upgrade());
        assert_eq!(payload.readall().unwrap().as_ref(), b"some raw data");
    }

    #[test]
    fn test_http_request_parser_utf8() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             x-test: тест\r\n\r\n");
        let (req, _) = parse_ready!(&mut buf);

        assert_eq!(req.headers().get("x-test").unwrap().as_bytes(),
                   "тест".as_bytes());
    }

    #[test]
    fn test_http_request_parser_two_slashes() {
        let mut buf = Buffer::new(
            "GET //path HTTP/1.1\r\n\r\n");
        let (req, _) = parse_ready!(&mut buf);

        assert_eq!(req.path(), "//path");
    }

    #[test]
    fn test_http_request_parser_bad_method() {
        let mut buf = Buffer::new(
            "!12%()+=~$ /get HTTP/1.1\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_parser_bad_version() {
        let mut buf = Buffer::new("GET //get HT/11\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_chunked_payload() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");

        let mut reader = Reader::new();
        let (req, mut payload) = reader_parse_ready!(reader.parse(&mut buf));
        assert!(req.chunked().unwrap());
        assert!(!payload.eof());

        buf.feed_data("4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n");
        not_ready!(reader.parse(&mut buf));
        assert!(!payload.eof());
        assert_eq!(payload.readall().unwrap().as_ref(), b"dataline");
        assert!(payload.eof());
    }

    #[test]
    fn test_http_request_chunked_payload_and_next_message() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");

        let mut reader = Reader::new();

        let (req, mut payload) = reader_parse_ready!(reader.parse(&mut buf));
        assert!(req.chunked().unwrap());
        assert!(!payload.eof());

        buf.feed_data(
            "4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n\
             POST /test2 HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");

        let (req2, payload2) = reader_parse_ready!(reader.parse(&mut buf));
        assert_eq!(*req2.method(), Method::POST);
        assert!(req2.chunked().unwrap());
        assert!(!payload2.eof());

        assert_eq!(payload.readall().unwrap().as_ref(), b"dataline");
        assert!(payload.eof());
    }

    #[test]
    fn test_http_request_chunked_payload_chunks() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");

        let mut reader = Reader::new();
        let (req, mut payload) = reader_parse_ready!(reader.parse(&mut buf));
        assert!(req.chunked().unwrap());
        assert!(!payload.eof());

        buf.feed_data("4\r\ndata\r");
        not_ready!(reader.parse(&mut buf));

        buf.feed_data("\n4");
        not_ready!(reader.parse(&mut buf));

        buf.feed_data("\r");
        not_ready!(reader.parse(&mut buf));
        buf.feed_data("\n");
        not_ready!(reader.parse(&mut buf));

        buf.feed_data("li");
        not_ready!(reader.parse(&mut buf));

        buf.feed_data("ne\r\n0\r\n");
        not_ready!(reader.parse(&mut buf));

        //buf.feed_data("test: test\r\n");
        //not_ready!(reader.parse(&mut buf));

        assert_eq!(payload.readall().unwrap().as_ref(), b"dataline");
        assert!(!payload.eof());

        buf.feed_data("\r\n");
        not_ready!(reader.parse(&mut buf));
        assert!(payload.eof());
    }

    #[test]
    fn test_parse_chunked_payload_chunk_extension() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n");

        let mut reader = Reader::new();
        let (req, mut payload) = reader_parse_ready!(reader.parse(&mut buf));
        assert!(req.chunked().unwrap());
        assert!(!payload.eof());

        buf.feed_data("4;test\r\ndata\r\n4\r\nline\r\n0\r\n\r\n"); // test: test\r\n\r\n")
        not_ready!(reader.parse(&mut buf));
        assert!(!payload.eof());
        assert_eq!(payload.readall().unwrap().as_ref(), b"dataline");
        assert!(payload.eof());
    }

    /*#[test]
    #[should_panic]
    fn test_parse_multiline() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             test: line\r\n \
               continue\r\n\
             test2: data\r\n\
             \r\n", false);

        let mut reader = Reader::new();
        match reader.parse(&mut buf) {
            Ok(res) => (),
            Err(err) => panic!("{:?}", err),
        }
    }*/
}
