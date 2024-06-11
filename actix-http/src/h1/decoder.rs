use std::{io, marker::PhantomData, mem::MaybeUninit, task::Poll};

use actix_codec::Decoder;
use bytes::{Bytes, BytesMut};
use http::{
    header::{self, HeaderName, HeaderValue},
    Method, StatusCode, Uri, Version,
};
use tracing::{debug, error, trace};

use super::chunked::ChunkedState;
use crate::{error::ParseError, header::HeaderMap, ConnectionType, Request, ResponseHead};

pub(crate) const MAX_BUFFER_SIZE: usize = 131_072;
const MAX_HEADERS: usize = 96;

/// Incoming message decoder
pub(crate) struct MessageDecoder<T: MessageType>(PhantomData<T>);

#[derive(Debug)]
/// Incoming request type
pub(crate) enum PayloadType {
    None,
    Payload(PayloadDecoder),
    Stream(PayloadDecoder),
}

impl<T: MessageType> Default for MessageDecoder<T> {
    fn default() -> Self {
        MessageDecoder(PhantomData)
    }
}

impl<T: MessageType> Decoder for MessageDecoder<T> {
    type Item = (T, PayloadType);
    type Error = ParseError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        T::decode(src)
    }
}

pub(crate) enum PayloadLength {
    Payload(PayloadType),
    UpgradeWebSocket,
    None,
}

impl PayloadLength {
    /// Returns true if variant is `None`.
    fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Returns true if variant is represents zero-length (not none) payload.
    fn is_zero(&self) -> bool {
        matches!(
            self,
            PayloadLength::Payload(PayloadType::Payload(PayloadDecoder {
                kind: Kind::Length(0)
            }))
        )
    }
}

pub(crate) trait MessageType: Sized {
    fn set_connection_type(&mut self, conn_type: Option<ConnectionType>);

    fn set_expect(&mut self);

    fn headers_mut(&mut self) -> &mut HeaderMap;

    fn decode(src: &mut BytesMut) -> Result<Option<(Self, PayloadType)>, ParseError>;

    fn set_headers(
        &mut self,
        slice: &Bytes,
        raw_headers: &[HeaderIndex],
        version: Version,
    ) -> Result<PayloadLength, ParseError> {
        let mut ka = None;
        let mut has_upgrade_websocket = false;
        let mut expect = false;
        let mut chunked = false;
        let mut seen_te = false;
        let mut content_length = None;

        {
            let headers = self.headers_mut();

            for idx in raw_headers.iter() {
                let name = HeaderName::from_bytes(&slice[idx.name.0..idx.name.1]).unwrap();

                // SAFETY: httparse already checks header value is only visible ASCII bytes
                // from_maybe_shared_unchecked contains debug assertions so they are omitted here
                let value = unsafe {
                    HeaderValue::from_maybe_shared_unchecked(slice.slice(idx.value.0..idx.value.1))
                };

                match name {
                    header::CONTENT_LENGTH if content_length.is_some() => {
                        debug!("multiple Content-Length");
                        return Err(ParseError::Header);
                    }

                    header::CONTENT_LENGTH => match value.to_str().map(str::trim) {
                        Ok(val) if val.starts_with('+') => {
                            debug!("illegal Content-Length: {:?}", val);
                            return Err(ParseError::Header);
                        }

                        Ok(val) => {
                            if let Ok(len) = val.parse::<u64>() {
                                // accept 0 lengths here and remove them in `decode` after all
                                // headers have been processed to prevent request smuggling issues
                                content_length = Some(len);
                            } else {
                                debug!("illegal Content-Length: {:?}", val);
                                return Err(ParseError::Header);
                            }
                        }

                        Err(_) => {
                            debug!("illegal Content-Length: {:?}", value);
                            return Err(ParseError::Header);
                        }
                    },

                    // transfer-encoding
                    header::TRANSFER_ENCODING if seen_te => {
                        debug!("multiple Transfer-Encoding not allowed");
                        return Err(ParseError::Header);
                    }

                    header::TRANSFER_ENCODING if version == Version::HTTP_11 => {
                        seen_te = true;

                        if let Ok(val) = value.to_str().map(str::trim) {
                            if val.eq_ignore_ascii_case("chunked") {
                                chunked = true;
                            } else if val.eq_ignore_ascii_case("identity") {
                                // allow silently since multiple TE headers are already checked
                            } else {
                                debug!("illegal Transfer-Encoding: {:?}", val);
                                return Err(ParseError::Header);
                            }
                        } else {
                            return Err(ParseError::Header);
                        }
                    }

                    // connection keep-alive state
                    header::CONNECTION => {
                        ka = if let Ok(conn) = value.to_str().map(str::trim) {
                            if conn.eq_ignore_ascii_case("keep-alive") {
                                Some(ConnectionType::KeepAlive)
                            } else if conn.eq_ignore_ascii_case("close") {
                                Some(ConnectionType::Close)
                            } else if conn.eq_ignore_ascii_case("upgrade") {
                                Some(ConnectionType::Upgrade)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                    }

                    header::UPGRADE => {
                        if let Ok(val) = value.to_str().map(str::trim) {
                            if val.eq_ignore_ascii_case("websocket") {
                                has_upgrade_websocket = true;
                            }
                        }
                    }

                    header::EXPECT => {
                        let bytes = value.as_bytes();
                        if bytes.len() >= 4 && &bytes[0..4] == b"100-" {
                            expect = true;
                        }
                    }

                    _ => {}
                }

                headers.append(name, value);
            }
        }

        self.set_connection_type(ka);

        if expect {
            self.set_expect()
        }

        // https://datatracker.ietf.org/doc/html/rfc7230#section-3.3.3
        if chunked {
            // Chunked encoding
            Ok(PayloadLength::Payload(PayloadType::Payload(
                PayloadDecoder::chunked(),
            )))
        } else if has_upgrade_websocket {
            Ok(PayloadLength::UpgradeWebSocket)
        } else if let Some(len) = content_length {
            // Content-Length
            Ok(PayloadLength::Payload(PayloadType::Payload(
                PayloadDecoder::length(len),
            )))
        } else {
            Ok(PayloadLength::None)
        }
    }
}

impl MessageType for Request {
    fn set_connection_type(&mut self, conn_type: Option<ConnectionType>) {
        if let Some(ctype) = conn_type {
            self.head_mut().set_connection_type(ctype);
        }
    }

    fn set_expect(&mut self) {
        self.head_mut().set_expect();
    }

    fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.head_mut().headers
    }

    fn decode(src: &mut BytesMut) -> Result<Option<(Self, PayloadType)>, ParseError> {
        let mut headers: [HeaderIndex; MAX_HEADERS] = EMPTY_HEADER_INDEX_ARRAY;

        let (len, method, uri, ver, h_len) = {
            // SAFETY:
            // Create an uninitialized array of `MaybeUninit`. The `assume_init` is safe because the
            // type we are claiming to have initialized here is a bunch of `MaybeUninit`s, which
            // do not require initialization.
            let mut parsed = unsafe {
                MaybeUninit::<[MaybeUninit<httparse::Header<'_>>; MAX_HEADERS]>::uninit()
                    .assume_init()
            };

            let mut req = httparse::Request::new(&mut []);

            match req.parse_with_uninit_headers(src, &mut parsed)? {
                httparse::Status::Complete(len) => {
                    let method = Method::from_bytes(req.method.unwrap().as_bytes())
                        .map_err(|_| ParseError::Method)?;
                    let uri = Uri::try_from(req.path.unwrap())?;
                    let version = if req.version.unwrap() == 1 {
                        Version::HTTP_11
                    } else {
                        Version::HTTP_10
                    };
                    HeaderIndex::record(src, req.headers, &mut headers);

                    (len, method, uri, version, req.headers.len())
                }

                httparse::Status::Partial => {
                    return if src.len() >= MAX_BUFFER_SIZE {
                        trace!("MAX_BUFFER_SIZE unprocessed data reached, closing");
                        Err(ParseError::TooLarge)
                    } else {
                        // Return None to notify more read are needed for parsing request
                        Ok(None)
                    };
                }
            }
        };

        let mut msg = Request::new();

        // convert headers
        let mut length = msg.set_headers(&src.split_to(len).freeze(), &headers[..h_len], ver)?;

        // disallow HTTP/1.0 POST requests that do not contain a Content-Length headers
        // see https://datatracker.ietf.org/doc/html/rfc1945#section-7.2.2
        if ver == Version::HTTP_10 && method == Method::POST && length.is_none() {
            debug!("no Content-Length specified for HTTP/1.0 POST request");
            return Err(ParseError::Header);
        }

        // Remove CL value if 0 now that all headers and HTTP/1.0 special cases are processed.
        // Protects against some request smuggling attacks.
        // See https://github.com/actix/actix-web/issues/2767.
        if length.is_zero() {
            length = PayloadLength::None;
        }

        // payload decoder
        let decoder = match length {
            PayloadLength::Payload(pl) => pl,
            PayloadLength::UpgradeWebSocket => {
                // upgrade (WebSocket)
                PayloadType::Stream(PayloadDecoder::eof())
            }
            PayloadLength::None => {
                if method == Method::CONNECT {
                    PayloadType::Stream(PayloadDecoder::eof())
                } else {
                    PayloadType::None
                }
            }
        };

        let head = msg.head_mut();
        head.uri = uri;
        head.method = method;
        head.version = ver;

        Ok(Some((msg, decoder)))
    }
}

impl MessageType for ResponseHead {
    fn set_connection_type(&mut self, conn_type: Option<ConnectionType>) {
        if let Some(ctype) = conn_type {
            ResponseHead::set_connection_type(self, ctype);
        }
    }

    fn set_expect(&mut self) {}

    fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    fn decode(src: &mut BytesMut) -> Result<Option<(Self, PayloadType)>, ParseError> {
        let mut headers: [HeaderIndex; MAX_HEADERS] = EMPTY_HEADER_INDEX_ARRAY;

        let (len, ver, status, h_len) = {
            // SAFETY:
            // Create an uninitialized array of `MaybeUninit`. The `assume_init` is safe because the
            // type we are claiming to have initialized here is a bunch of `MaybeUninit`s, which
            // do not require initialization.
            let mut parsed = unsafe {
                MaybeUninit::<[MaybeUninit<httparse::Header<'_>>; MAX_HEADERS]>::uninit()
                    .assume_init()
            };

            let mut res = httparse::Response::new(&mut []);

            let mut config = httparse::ParserConfig::default();
            config.allow_spaces_after_header_name_in_responses(true);

            match config.parse_response_with_uninit_headers(&mut res, src, &mut parsed)? {
                httparse::Status::Complete(len) => {
                    let version = if res.version.unwrap() == 1 {
                        Version::HTTP_11
                    } else {
                        Version::HTTP_10
                    };

                    let status =
                        StatusCode::from_u16(res.code.unwrap()).map_err(|_| ParseError::Status)?;
                    HeaderIndex::record(src, res.headers, &mut headers);

                    (len, version, status, res.headers.len())
                }

                httparse::Status::Partial => {
                    return if src.len() >= MAX_BUFFER_SIZE {
                        error!("MAX_BUFFER_SIZE unprocessed data reached, closing");
                        Err(ParseError::TooLarge)
                    } else {
                        Ok(None)
                    }
                }
            }
        };

        let mut msg = ResponseHead::new(status);
        msg.version = ver;

        // convert headers
        let mut length = msg.set_headers(&src.split_to(len).freeze(), &headers[..h_len], ver)?;

        // Remove CL value if 0 now that all headers and HTTP/1.0 special cases are processed.
        // Protects against some request smuggling attacks.
        // See https://github.com/actix/actix-web/issues/2767.
        if length.is_zero() {
            length = PayloadLength::None;
        }

        // message payload
        let decoder = if let PayloadLength::Payload(pl) = length {
            pl
        } else if status == StatusCode::SWITCHING_PROTOCOLS {
            // switching protocol or connect
            PayloadType::Stream(PayloadDecoder::eof())
        } else {
            // for HTTP/1.0 read to eof and close connection
            if msg.version == Version::HTTP_10 {
                msg.set_connection_type(ConnectionType::Close);
                PayloadType::Payload(PayloadDecoder::eof())
            } else {
                PayloadType::None
            }
        };

        Ok(Some((msg, decoder)))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct HeaderIndex {
    pub(crate) name: (usize, usize),
    pub(crate) value: (usize, usize),
}

pub(crate) const EMPTY_HEADER_INDEX: HeaderIndex = HeaderIndex {
    name: (0, 0),
    value: (0, 0),
};

pub(crate) const EMPTY_HEADER_INDEX_ARRAY: [HeaderIndex; MAX_HEADERS] =
    [EMPTY_HEADER_INDEX; MAX_HEADERS];

impl HeaderIndex {
    pub(crate) fn record(
        bytes: &[u8],
        headers: &[httparse::Header<'_>],
        indices: &mut [HeaderIndex],
    ) {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Chunk type yielded while decoding a payload.
pub enum PayloadItem {
    Chunk(Bytes),
    Eof,
}

/// Decoder that can handle different payload types.
///
/// If a message body does not use `Transfer-Encoding`, it should include a `Content-Length`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadDecoder {
    kind: Kind,
}

impl PayloadDecoder {
    /// Constructs a fixed-length payload decoder.
    pub fn length(x: u64) -> PayloadDecoder {
        PayloadDecoder {
            kind: Kind::Length(x),
        }
    }

    /// Constructs a chunked encoding decoder.
    pub fn chunked() -> PayloadDecoder {
        PayloadDecoder {
            kind: Kind::Chunked(ChunkedState::Size, 0),
        }
    }

    /// Creates an decoder that yields chunks until the stream returns EOF.
    pub fn eof() -> PayloadDecoder {
        PayloadDecoder { kind: Kind::Eof }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// A reader used when a `Content-Length` header is passed with a positive integer.
    Length(u64),

    /// A reader used when `Transfer-Encoding` is `chunked`.
    Chunked(ChunkedState, u64),

    /// A reader used for responses that don't indicate a length or chunked.
    ///
    /// Note: This should only used for `Response`s. It is illegal for a `Request` to be made
    /// without either of `Content-Length` and `Transfer-Encoding: chunked` missing, as explained
    /// in [RFC 7230 §3.3.3]:
    ///
    /// > If a Transfer-Encoding header field is present in a response and the chunked transfer
    /// > coding is not the final encoding, the message body length is determined by reading the
    /// > connection until it is closed by the server. If a Transfer-Encoding header field is
    /// > present in a request and the chunked transfer coding is not the final encoding, the
    /// > message body length cannot be determined reliably; the server MUST respond with the 400
    /// > (Bad Request) status code and then close the connection.
    ///
    /// [RFC 7230 §3.3.3]: https://datatracker.ietf.org/doc/html/rfc7230#section-3.3.3
    Eof,
}

impl Decoder for PayloadDecoder {
    type Item = PayloadItem;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.kind {
            Kind::Length(ref mut remaining) => {
                if *remaining == 0 {
                    Ok(Some(PayloadItem::Eof))
                } else {
                    if src.is_empty() {
                        return Ok(None);
                    }
                    let len = src.len() as u64;
                    let buf;
                    if *remaining > len {
                        buf = src.split().freeze();
                        *remaining -= len;
                    } else {
                        buf = src.split_to(*remaining as usize).freeze();
                        *remaining = 0;
                    };
                    trace!("Length read: {}", buf.len());
                    Ok(Some(PayloadItem::Chunk(buf)))
                }
            }

            Kind::Chunked(ref mut state, ref mut size) => {
                loop {
                    let mut buf = None;

                    // advances the chunked state
                    *state = match state.step(src, size, &mut buf) {
                        Poll::Pending => return Ok(None),
                        Poll::Ready(Ok(state)) => state,
                        Poll::Ready(Err(err)) => return Err(err),
                    };

                    if *state == ChunkedState::End {
                        trace!("End of chunked stream");
                        return Ok(Some(PayloadItem::Eof));
                    }

                    if let Some(buf) = buf {
                        return Ok(Some(PayloadItem::Chunk(buf)));
                    }

                    if src.is_empty() {
                        return Ok(None);
                    }
                }
            }

            Kind::Eof => {
                if src.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(PayloadItem::Chunk(src.split().freeze())))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{header::SET_COOKIE, HttpMessage as _};

    impl PayloadType {
        pub(crate) fn unwrap(self) -> PayloadDecoder {
            match self {
                PayloadType::Payload(pl) => pl,
                _ => panic!(),
            }
        }

        pub(crate) fn is_unhandled(&self) -> bool {
            matches!(self, PayloadType::Stream(_))
        }
    }

    impl PayloadItem {
        pub(crate) fn chunk(self) -> Bytes {
            match self {
                PayloadItem::Chunk(chunk) => chunk,
                _ => panic!("error"),
            }
        }

        pub(crate) fn eof(&self) -> bool {
            matches!(*self, PayloadItem::Eof)
        }
    }

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
    fn test_parse() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\n\r\n");

        let mut reader = MessageDecoder::<Request>::default();
        match reader.decode(&mut buf) {
            Ok(Some((req, _))) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_partial() {
        let mut buf = BytesMut::from("PUT /test HTTP/1");

        let mut reader = MessageDecoder::<Request>::default();
        assert!(reader.decode(&mut buf).unwrap().is_none());

        buf.extend(b".1\r\n\r\n");
        let (req, _) = reader.decode(&mut buf).unwrap().unwrap();
        assert_eq!(req.version(), Version::HTTP_11);
        assert_eq!(*req.method(), Method::PUT);
        assert_eq!(req.path(), "/test");
    }

    #[test]
    fn parse_h09_reject() {
        let mut buf = BytesMut::from(
            "GET /test1 HTTP/0.9\r\n\
            \r\n",
        );

        let mut reader = MessageDecoder::<Request>::default();
        reader.decode(&mut buf).unwrap_err();

        let mut buf = BytesMut::from(
            "POST /test2 HTTP/0.9\r\n\
            Content-Length: 3\r\n\
            \r\n
            abc",
        );

        let mut reader = MessageDecoder::<Request>::default();
        reader.decode(&mut buf).unwrap_err();
    }

    #[test]
    fn parse_h10_get() {
        let mut buf = BytesMut::from(
            "GET /test1 HTTP/1.0\r\n\
            \r\n",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let (req, _) = reader.decode(&mut buf).unwrap().unwrap();
        assert_eq!(req.version(), Version::HTTP_10);
        assert_eq!(*req.method(), Method::GET);
        assert_eq!(req.path(), "/test1");

        let mut buf = BytesMut::from(
            "GET /test2 HTTP/1.0\r\n\
            Content-Length: 0\r\n\
            \r\n",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let (req, _) = reader.decode(&mut buf).unwrap().unwrap();
        assert_eq!(req.version(), Version::HTTP_10);
        assert_eq!(*req.method(), Method::GET);
        assert_eq!(req.path(), "/test2");

        let mut buf = BytesMut::from(
            "GET /test3 HTTP/1.0\r\n\
            Content-Length: 3\r\n\
            \r\n
            abc",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let (req, _) = reader.decode(&mut buf).unwrap().unwrap();
        assert_eq!(req.version(), Version::HTTP_10);
        assert_eq!(*req.method(), Method::GET);
        assert_eq!(req.path(), "/test3");
    }

    #[test]
    fn parse_h10_post() {
        let mut buf = BytesMut::from(
            "POST /test1 HTTP/1.0\r\n\
            Content-Length: 3\r\n\
            \r\n\
            abc",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let (req, _) = reader.decode(&mut buf).unwrap().unwrap();
        assert_eq!(req.version(), Version::HTTP_10);
        assert_eq!(*req.method(), Method::POST);
        assert_eq!(req.path(), "/test1");

        let mut buf = BytesMut::from(
            "POST /test2 HTTP/1.0\r\n\
            Content-Length: 0\r\n\
            \r\n",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let (req, _) = reader.decode(&mut buf).unwrap().unwrap();
        assert_eq!(req.version(), Version::HTTP_10);
        assert_eq!(*req.method(), Method::POST);
        assert_eq!(req.path(), "/test2");

        let mut buf = BytesMut::from(
            "POST /test3 HTTP/1.0\r\n\
            \r\n",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let err = reader.decode(&mut buf).unwrap_err();
        assert!(err.to_string().contains("Header"))
    }

    #[test]
    fn test_parse_body() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");

        let mut reader = MessageDecoder::<Request>::default();
        let (req, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();
        assert_eq!(req.version(), Version::HTTP_11);
        assert_eq!(*req.method(), Method::GET);
        assert_eq!(req.path(), "/test");
        assert_eq!(
            pl.decode(&mut buf).unwrap().unwrap().chunk().as_ref(),
            b"body"
        );
    }

    #[test]
    fn test_parse_body_crlf() {
        let mut buf = BytesMut::from("\r\nGET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");

        let mut reader = MessageDecoder::<Request>::default();
        let (req, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();
        assert_eq!(req.version(), Version::HTTP_11);
        assert_eq!(*req.method(), Method::GET);
        assert_eq!(req.path(), "/test");
        assert_eq!(
            pl.decode(&mut buf).unwrap().unwrap().chunk().as_ref(),
            b"body"
        );
    }

    #[test]
    fn test_parse_partial_eof() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\n");
        let mut reader = MessageDecoder::<Request>::default();
        assert!(reader.decode(&mut buf).unwrap().is_none());

        buf.extend(b"\r\n");
        let (req, _) = reader.decode(&mut buf).unwrap().unwrap();
        assert_eq!(req.version(), Version::HTTP_11);
        assert_eq!(*req.method(), Method::GET);
        assert_eq!(req.path(), "/test");
    }

    #[test]
    fn test_headers_split_field() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\n");

        let mut reader = MessageDecoder::<Request>::default();
        assert! { reader.decode(&mut buf).unwrap().is_none() }

        buf.extend(b"t");
        assert! { reader.decode(&mut buf).unwrap().is_none() }

        buf.extend(b"es");
        assert! { reader.decode(&mut buf).unwrap().is_none() }

        buf.extend(b"t: value\r\n\r\n");
        let (req, _) = reader.decode(&mut buf).unwrap().unwrap();
        assert_eq!(req.version(), Version::HTTP_11);
        assert_eq!(*req.method(), Method::GET);
        assert_eq!(req.path(), "/test");
        assert_eq!(
            req.headers()
                .get(HeaderName::try_from("test").unwrap())
                .unwrap()
                .as_bytes(),
            b"value"
        );
    }

    #[test]
    fn test_headers_multi_value() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             Set-Cookie: c1=cookie1\r\n\
             Set-Cookie: c2=cookie2\r\n\r\n",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let (req, _) = reader.decode(&mut buf).unwrap().unwrap();

        let val: Vec<_> = req
            .headers()
            .get_all(SET_COOKIE)
            .map(|v| v.to_str().unwrap().to_owned())
            .collect();
        assert_eq!(val[0], "c1=cookie1");
        assert_eq!(val[1], "c2=cookie2");
    }

    #[test]
    fn test_conn_default_1_0() {
        let req = parse_ready!(&mut BytesMut::from("GET /test HTTP/1.0\r\n\r\n"));
        assert_eq!(req.head().connection_type(), ConnectionType::Close);
    }

    #[test]
    fn test_conn_default_1_1() {
        let req = parse_ready!(&mut BytesMut::from("GET /test HTTP/1.1\r\n\r\n"));
        assert_eq!(req.head().connection_type(), ConnectionType::KeepAlive);
    }

    #[test]
    fn test_conn_close() {
        let req = parse_ready!(&mut BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: close\r\n\r\n",
        ));
        assert_eq!(req.head().connection_type(), ConnectionType::Close);

        let req = parse_ready!(&mut BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: Close\r\n\r\n",
        ));
        assert_eq!(req.head().connection_type(), ConnectionType::Close);
    }

    #[test]
    fn test_conn_close_1_0() {
        let req = parse_ready!(&mut BytesMut::from(
            "GET /test HTTP/1.0\r\n\
             connection: close\r\n\r\n",
        ));
        assert_eq!(req.head().connection_type(), ConnectionType::Close);
    }

    #[test]
    fn test_conn_keep_alive_1_0() {
        let req = parse_ready!(&mut BytesMut::from(
            "GET /test HTTP/1.0\r\n\
             connection: keep-alive\r\n\r\n",
        ));
        assert_eq!(req.head().connection_type(), ConnectionType::KeepAlive);

        let req = parse_ready!(&mut BytesMut::from(
            "GET /test HTTP/1.0\r\n\
             connection: Keep-Alive\r\n\r\n",
        ));
        assert_eq!(req.head().connection_type(), ConnectionType::KeepAlive);
    }

    #[test]
    fn test_conn_keep_alive_1_1() {
        let req = parse_ready!(&mut BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: keep-alive\r\n\r\n",
        ));
        assert_eq!(req.head().connection_type(), ConnectionType::KeepAlive);
    }

    #[test]
    fn test_conn_other_1_0() {
        let req = parse_ready!(&mut BytesMut::from(
            "GET /test HTTP/1.0\r\n\
             connection: other\r\n\r\n",
        ));
        assert_eq!(req.head().connection_type(), ConnectionType::Close);
    }

    #[test]
    fn test_conn_other_1_1() {
        let req = parse_ready!(&mut BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: other\r\n\r\n",
        ));
        assert_eq!(req.head().connection_type(), ConnectionType::KeepAlive);
    }

    #[test]
    fn test_conn_upgrade() {
        let req = parse_ready!(&mut BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             upgrade: websockets\r\n\
             connection: upgrade\r\n\r\n",
        ));

        assert!(req.upgrade());
        assert_eq!(req.head().connection_type(), ConnectionType::Upgrade);

        let req = parse_ready!(&mut BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             upgrade: Websockets\r\n\
             connection: Upgrade\r\n\r\n",
        ));

        assert!(req.upgrade());
        assert_eq!(req.head().connection_type(), ConnectionType::Upgrade);
    }

    #[test]
    fn test_conn_upgrade_connect_method() {
        let req = parse_ready!(&mut BytesMut::from(
            "CONNECT /test HTTP/1.1\r\n\
             content-type: text/plain\r\n\r\n",
        ));

        assert!(req.upgrade());
    }

    #[test]
    fn test_headers_bad_content_length() {
        // string CL
        expect_parse_err!(&mut BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             content-length: line\r\n\r\n",
        ));

        // negative CL
        expect_parse_err!(&mut BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             content-length: -1\r\n\r\n",
        ));
    }

    #[test]
    fn octal_ish_cl_parsed_as_decimal() {
        let mut buf = BytesMut::from(
            "POST /test HTTP/1.1\r\n\
             content-length: 011\r\n\r\n",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let (_req, pl) = reader.decode(&mut buf).unwrap().unwrap();
        assert!(matches!(
            pl,
            PayloadType::Payload(pl) if pl == PayloadDecoder::length(11)
        ));
    }

    #[test]
    fn test_invalid_header() {
        expect_parse_err!(&mut BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             test line\r\n\r\n",
        ));
    }

    #[test]
    fn test_invalid_name() {
        expect_parse_err!(&mut BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             test[]: line\r\n\r\n",
        ));
    }

    #[test]
    fn test_http_request_bad_status_line() {
        expect_parse_err!(&mut BytesMut::from("getpath \r\n\r\n"));
    }

    #[test]
    fn test_http_request_upgrade_websocket() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: upgrade\r\n\
             upgrade: websocket\r\n\r\n\
             some raw data",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let (req, pl) = reader.decode(&mut buf).unwrap().unwrap();
        assert_eq!(req.head().connection_type(), ConnectionType::Upgrade);
        assert!(req.upgrade());
        assert!(pl.is_unhandled());
    }

    #[test]
    fn test_http_request_upgrade_h2c() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: upgrade, http2-settings\r\n\
             upgrade: h2c\r\n\
             http2-settings: dummy\r\n\r\n",
        );
        let mut reader = MessageDecoder::<Request>::default();
        let (req, pl) = reader.decode(&mut buf).unwrap().unwrap();
        // `connection: upgrade, http2-settings` doesn't work properly..
        // see MessageType::set_headers().
        //
        // The line below should be:
        // assert_eq!(req.head().connection_type(), ConnectionType::Upgrade);
        assert_eq!(req.head().connection_type(), ConnectionType::KeepAlive);
        assert!(req.upgrade());
        assert!(!pl.is_unhandled());
    }

    #[test]
    fn test_http_request_parser_utf8() {
        let req = parse_ready!(&mut BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             x-test: тест\r\n\r\n",
        ));

        assert_eq!(
            req.headers().get("x-test").unwrap().as_bytes(),
            "тест".as_bytes()
        );
    }

    #[test]
    fn test_http_request_parser_two_slashes() {
        let req = parse_ready!(&mut BytesMut::from("GET //path HTTP/1.1\r\n\r\n"));
        assert_eq!(req.path(), "//path");
    }

    #[test]
    fn test_http_request_parser_bad_method() {
        expect_parse_err!(&mut BytesMut::from("!12%()+=~$ /get HTTP/1.1\r\n\r\n"));
    }

    #[test]
    fn test_http_request_parser_bad_version() {
        expect_parse_err!(&mut BytesMut::from("GET //get HT/11\r\n\r\n"));
    }

    #[test]
    fn test_response_http10_read_until_eof() {
        let mut buf = BytesMut::from("HTTP/1.0 200 Ok\r\n\r\ntest data");

        let mut reader = MessageDecoder::<ResponseHead>::default();
        let (_msg, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();

        let chunk = pl.decode(&mut buf).unwrap().unwrap();
        assert_eq!(chunk, PayloadItem::Chunk(Bytes::from_static(b"test data")));
    }

    #[test]
    fn hrs_multiple_content_length() {
        expect_parse_err!(&mut BytesMut::from(
            "GET / HTTP/1.1\r\n\
            Host: example.com\r\n\
            Content-Length: 4\r\n\
            Content-Length: 2\r\n\
            \r\n\
            abcd",
        ));

        expect_parse_err!(&mut BytesMut::from(
            "GET / HTTP/1.1\r\n\
            Host: example.com\r\n\
            Content-Length: 0\r\n\
            Content-Length: 2\r\n\
            \r\n\
            ab",
        ));
    }

    #[test]
    fn hrs_content_length_plus() {
        expect_parse_err!(&mut BytesMut::from(
            "GET / HTTP/1.1\r\n\
            Host: example.com\r\n\
            Content-Length: +3\r\n\
            \r\n\
            000",
        ));
    }

    #[test]
    fn hrs_te_http10() {
        // in HTTP/1.0 transfer encoding is ignored and must therefore contain a CL header

        expect_parse_err!(&mut BytesMut::from(
            "POST / HTTP/1.0\r\n\
            Host: example.com\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            3\r\n\
            aaa\r\n\
            0\r\n\
            ",
        ));
    }

    #[test]
    fn hrs_cl_and_te_http10() {
        // in HTTP/1.0 transfer encoding is simply ignored so it's fine to have both

        let mut buf = BytesMut::from(
            "GET / HTTP/1.0\r\n\
            Host: example.com\r\n\
            Content-Length: 3\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            000",
        );

        parse_ready!(&mut buf);
    }

    #[test]
    fn hrs_unknown_transfer_encoding() {
        let mut buf = BytesMut::from(
            "GET / HTTP/1.1\r\n\
            Host: example.com\r\n\
            Transfer-Encoding: JUNK\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            5\r\n\
            hello\r\n\
            0",
        );

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn hrs_multiple_transfer_encoding() {
        let mut buf = BytesMut::from(
            "GET / HTTP/1.1\r\n\
            Host: example.com\r\n\
            Content-Length: 51\r\n\
            Transfer-Encoding: identity\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            0\r\n\
            \r\n\
            GET /forbidden HTTP/1.1\r\n\
            Host: example.com\r\n\r\n",
        );

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn transfer_encoding_agrees() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
            Host: example.com\r\n\
            Content-Length: 3\r\n\
            Transfer-Encoding: identity\r\n\
            \r\n\
            0\r\n",
        );

        let mut reader = MessageDecoder::<Request>::default();
        let (_msg, pl) = reader.decode(&mut buf).unwrap().unwrap();
        let mut pl = pl.unwrap();

        let chunk = pl.decode(&mut buf).unwrap().unwrap();
        assert_eq!(chunk, PayloadItem::Chunk(Bytes::from_static(b"0\r\n")));
    }
}
