//! Multipart response payload support.

use std::{
    cell::RefCell,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_web::{
    dev,
    error::{ParseError, PayloadError},
    http::header::{self, ContentDisposition, HeaderMap, HeaderName, HeaderValue},
    web::Bytes,
    HttpRequest,
};
use futures_core::stream::Stream;
use mime::Mime;

use crate::{
    error::Error,
    field::InnerField,
    payload::{PayloadBuffer, PayloadRef},
    safety::Safety,
    Field,
};

const MAX_HEADERS: usize = 32;

/// The server-side implementation of `multipart/form-data` requests.
///
/// This will parse the incoming stream into `MultipartItem` instances via its `Stream`
/// implementation. `MultipartItem::Field` contains multipart field. `MultipartItem::Multipart` is
/// used for nested multipart streams.
pub struct Multipart {
    flow: Flow,
    safety: Safety,
}

enum Flow {
    InFlight(Inner),

    /// Error container is Some until an error is returned out of the flow.
    Error(Option<Error>),
}

impl Multipart {
    /// Creates multipart instance from parts.
    pub fn new<S>(headers: &HeaderMap, stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, PayloadError>> + 'static,
    {
        match Self::find_ct_and_boundary(headers) {
            Ok((ct, boundary)) => Self::from_ct_and_boundary(ct, boundary, stream),
            Err(err) => Self::from_error(err),
        }
    }

    /// Creates multipart instance from parts.
    pub(crate) fn from_req(req: &HttpRequest, payload: &mut dev::Payload) -> Self {
        match Self::find_ct_and_boundary(req.headers()) {
            Ok((ct, boundary)) => Self::from_ct_and_boundary(ct, boundary, payload.take()),
            Err(err) => Self::from_error(err),
        }
    }

    /// Extract Content-Type and boundary info from headers.
    pub(crate) fn find_ct_and_boundary(headers: &HeaderMap) -> Result<(Mime, String), Error> {
        let content_type = headers
            .get(&header::CONTENT_TYPE)
            .ok_or(Error::ContentTypeMissing)?
            .to_str()
            .ok()
            .and_then(|content_type| content_type.parse::<Mime>().ok())
            .ok_or(Error::ContentTypeParse)?;

        if content_type.type_() != mime::MULTIPART {
            return Err(Error::ContentTypeIncompatible);
        }

        let boundary = content_type
            .get_param(mime::BOUNDARY)
            .ok_or(Error::BoundaryMissing)?
            .as_str()
            .to_owned();

        Ok((content_type, boundary))
    }

    /// Constructs a new multipart reader from given Content-Type, boundary, and stream.
    pub(crate) fn from_ct_and_boundary<S>(ct: Mime, boundary: String, stream: S) -> Multipart
    where
        S: Stream<Item = Result<Bytes, PayloadError>> + 'static,
    {
        Multipart {
            safety: Safety::new(),
            flow: Flow::InFlight(Inner {
                payload: PayloadRef::new(PayloadBuffer::new(stream)),
                content_type: ct,
                boundary,
                state: State::FirstBoundary,
                item: Item::None,
            }),
        }
    }

    /// Constructs a new multipart reader from given `MultipartError`.
    pub(crate) fn from_error(err: Error) -> Multipart {
        Multipart {
            flow: Flow::Error(Some(err)),
            safety: Safety::new(),
        }
    }

    /// Return requests parsed Content-Type or raise the stored error.
    pub(crate) fn content_type_or_bail(&mut self) -> Result<mime::Mime, Error> {
        match self.flow {
            Flow::InFlight(ref inner) => Ok(inner.content_type.clone()),
            Flow::Error(ref mut err) => Err(err
                .take()
                .expect("error should not be taken after it was returned")),
        }
    }
}

impl Stream for Multipart {
    type Item = Result<Field, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        match this.flow {
            Flow::InFlight(ref mut inner) => {
                if let Some(mut buffer) = inner.payload.get_mut(&this.safety) {
                    // check safety and poll read payload to buffer.
                    buffer.poll_stream(cx)?;
                } else if !this.safety.is_clean() {
                    // safety violation
                    return Poll::Ready(Some(Err(Error::NotConsumed)));
                } else {
                    return Poll::Pending;
                }

                inner.poll(&this.safety, cx)
            }

            Flow::Error(ref mut err) => Poll::Ready(Some(Err(err
                .take()
                .expect("Multipart polled after finish")))),
        }
    }
}

#[derive(PartialEq, Debug)]
enum State {
    /// Skip data until first boundary.
    FirstBoundary,

    /// Reading boundary.
    Boundary,

    /// Reading Headers.
    Headers,

    /// Stream EOF.
    Eof,
}

enum Item {
    None,
    Field(Rc<RefCell<InnerField>>),
}

struct Inner {
    /// Request's payload stream & buffer.
    payload: PayloadRef,

    /// Request's Content-Type.
    ///
    /// Guaranteed to have "multipart" top-level media type, i.e., `multipart/*`.
    content_type: Mime,

    /// Field boundary.
    boundary: String,

    state: State,
    item: Item,
}

impl Inner {
    fn read_field_headers(payload: &mut PayloadBuffer) -> Result<Option<HeaderMap>, Error> {
        match payload.read_until(b"\r\n\r\n")? {
            None => {
                if payload.eof {
                    Err(Error::Incomplete)
                } else {
                    Ok(None)
                }
            }

            Some(bytes) => {
                let mut hdrs = [httparse::EMPTY_HEADER; MAX_HEADERS];

                match httparse::parse_headers(&bytes, &mut hdrs).map_err(ParseError::from)? {
                    httparse::Status::Complete((_, hdrs)) => {
                        // convert headers
                        let mut headers = HeaderMap::with_capacity(hdrs.len());

                        for h in hdrs {
                            let name =
                                HeaderName::try_from(h.name).map_err(|_| ParseError::Header)?;
                            let value =
                                HeaderValue::try_from(h.value).map_err(|_| ParseError::Header)?;
                            headers.append(name, value);
                        }

                        Ok(Some(headers))
                    }

                    httparse::Status::Partial => Err(ParseError::Header.into()),
                }
            }
        }
    }

    /// Reads a field boundary from the payload buffer (and discards it).
    ///
    /// Reads "in-between" and "final" boundaries. E.g. for boundary = "foo":
    ///
    /// ```plain
    /// --foo    <-- in-between fields
    /// --foo--  <-- end of request body, should be followed by EOF
    /// ```
    ///
    /// Returns:
    ///
    /// - `Ok(Some(true))` - final field boundary read (EOF)
    /// - `Ok(Some(false))` - field boundary read
    /// - `Ok(None)` - boundary not found, more data needs reading
    /// - `Err(BoundaryMissing)` - multipart boundary is missing
    fn read_boundary(payload: &mut PayloadBuffer, boundary: &str) -> Result<Option<bool>, Error> {
        // TODO: need to read epilogue
        let chunk = match payload.readline_or_eof()? {
            // TODO: this might be okay as a let Some() else return Ok(None)
            None => return Ok(payload.eof.then_some(true)),
            Some(chunk) => chunk,
        };

        const BOUNDARY_MARKER: &[u8] = b"--";
        const LINE_BREAK: &[u8] = b"\r\n";

        let boundary_len = boundary.len();

        if chunk.len() < boundary_len + 2 + 2
            || !chunk.starts_with(BOUNDARY_MARKER)
            || &chunk[2..boundary_len + 2] != boundary.as_bytes()
        {
            return Err(Error::BoundaryMissing);
        }

        // chunk facts:
        // - long enough to contain boundary + 2 markers or 1 marker and line-break
        // - starts with boundary marker
        // - chunk contains correct boundary

        if &chunk[boundary_len + 2..] == LINE_BREAK {
            // boundary is followed by line-break, indicating more fields to come
            return Ok(Some(false));
        }

        // boundary is followed by marker
        if &chunk[boundary_len + 2..boundary_len + 4] == BOUNDARY_MARKER
            && (
                // chunk is exactly boundary len + 2 markers
                chunk.len() == boundary_len + 2 + 2
                // final boundary is allowed to end with a line-break
                || &chunk[boundary_len + 4..] == LINE_BREAK
            )
        {
            return Ok(Some(true));
        }

        Err(Error::BoundaryMissing)
    }

    fn skip_until_boundary(
        payload: &mut PayloadBuffer,
        boundary: &str,
    ) -> Result<Option<bool>, Error> {
        let mut eof = false;

        loop {
            match payload.readline()? {
                Some(chunk) => {
                    if chunk.is_empty() {
                        return Err(Error::BoundaryMissing);
                    }
                    if chunk.len() < boundary.len() {
                        continue;
                    }
                    if &chunk[..2] == b"--" && &chunk[2..chunk.len() - 2] == boundary.as_bytes() {
                        break;
                    } else {
                        if chunk.len() < boundary.len() + 2 {
                            continue;
                        }
                        let b: &[u8] = boundary.as_ref();
                        if &chunk[..boundary.len()] == b
                            && &chunk[boundary.len()..boundary.len() + 2] == b"--"
                        {
                            eof = true;
                            break;
                        }
                    }
                }
                None => {
                    return if payload.eof {
                        Err(Error::Incomplete)
                    } else {
                        Ok(None)
                    };
                }
            }
        }
        Ok(Some(eof))
    }

    fn poll(&mut self, safety: &Safety, cx: &Context<'_>) -> Poll<Option<Result<Field, Error>>> {
        if self.state == State::Eof {
            Poll::Ready(None)
        } else {
            // release field
            loop {
                // Nested multipart streams of fields has to be consumed
                // before switching to next
                if safety.current() {
                    let stop = match self.item {
                        Item::Field(ref mut field) => match field.borrow_mut().poll(safety) {
                            Poll::Pending => return Poll::Pending,
                            Poll::Ready(Some(Ok(_))) => continue,
                            Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err))),
                            Poll::Ready(None) => true,
                        },
                        Item::None => false,
                    };
                    if stop {
                        self.item = Item::None;
                    }
                    if let Item::None = self.item {
                        break;
                    }
                }
            }

            let field_headers = if let Some(mut payload) = self.payload.get_mut(safety) {
                match self.state {
                    // read until first boundary
                    State::FirstBoundary => {
                        match Inner::skip_until_boundary(&mut payload, &self.boundary)? {
                            None => return Poll::Pending,
                            Some(eof) => {
                                if eof {
                                    self.state = State::Eof;
                                    return Poll::Ready(None);
                                } else {
                                    self.state = State::Headers;
                                }
                            }
                        }
                    }

                    // read boundary
                    State::Boundary => match Inner::read_boundary(&mut payload, &self.boundary)? {
                        None => return Poll::Pending,
                        Some(eof) => {
                            if eof {
                                self.state = State::Eof;
                                return Poll::Ready(None);
                            } else {
                                self.state = State::Headers;
                            }
                        }
                    },

                    _ => {}
                }

                // read field headers for next field
                if self.state == State::Headers {
                    if let Some(headers) = Inner::read_field_headers(&mut payload)? {
                        self.state = State::Boundary;
                        headers
                    } else {
                        return Poll::Pending;
                    }
                } else {
                    unreachable!()
                }
            } else {
                log::debug!("NotReady: field is in flight");
                return Poll::Pending;
            };

            let field_content_disposition = field_headers
                .get(&header::CONTENT_DISPOSITION)
                .and_then(|cd| ContentDisposition::from_raw(cd).ok())
                .filter(|content_disposition| {
                    matches!(
                        content_disposition.disposition,
                        header::DispositionType::FormData,
                    )
                });

            let form_field_name = if self.content_type.subtype() == mime::FORM_DATA {
                // According to RFC 7578 §4.2, which relates to "multipart/form-data" requests
                // specifically, fields must have a Content-Disposition header, its disposition
                // type must be set as "form-data", and it must have a name parameter.

                let Some(cd) = &field_content_disposition else {
                    return Poll::Ready(Some(Err(Error::ContentDispositionMissing)));
                };

                let Some(field_name) = cd.get_name() else {
                    return Poll::Ready(Some(Err(Error::ContentDispositionNameMissing)));
                };

                Some(field_name.to_owned())
            } else {
                None
            };

            // TODO: check out other multipart/* RFCs for specific requirements

            let field_content_type: Option<Mime> = field_headers
                .get(&header::CONTENT_TYPE)
                .and_then(|ct| ct.to_str().ok())
                .and_then(|ct| ct.parse().ok());

            self.state = State::Boundary;

            // nested multipart stream is not supported
            if let Some(mime) = &field_content_type {
                if mime.type_() == mime::MULTIPART {
                    return Poll::Ready(Some(Err(Error::Nested)));
                }
            }

            let field_inner =
                InnerField::new_in_rc(self.payload.clone(), self.boundary.clone(), &field_headers)?;

            self.item = Item::Field(Rc::clone(&field_inner));

            Poll::Ready(Some(Ok(Field::new(
                field_content_type,
                field_content_disposition,
                form_field_name,
                field_headers,
                safety.clone(cx),
                field_inner,
            ))))
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // InnerMultipartItem::Field has to be dropped first because of Safety.
        self.item = Item::None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use actix_http::h1;
    use actix_web::{
        http::header::{DispositionParam, DispositionType},
        rt,
        test::TestRequest,
        web::{BufMut as _, BytesMut},
        FromRequest,
    };
    use assert_matches::assert_matches;
    use futures_test::stream::StreamTestExt as _;
    use futures_util::{stream, StreamExt as _};
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    use super::*;

    const BOUNDARY: &str = "abbc761f78ff4d7cb7573b5a23f96ef0";

    #[actix_rt::test]
    async fn test_boundary() {
        let headers = HeaderMap::new();
        match Multipart::find_ct_and_boundary(&headers) {
            Err(Error::ContentTypeMissing) => {}
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("test"),
        );

        match Multipart::find_ct_and_boundary(&headers) {
            Err(Error::ContentTypeParse) => {}
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("multipart/mixed"),
        );
        match Multipart::find_ct_and_boundary(&headers) {
            Err(Error::BoundaryMissing) => {}
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(
                "multipart/mixed; boundary=\"5c02368e880e436dab70ed54e1c58209\"",
            ),
        );

        assert_eq!(
            Multipart::find_ct_and_boundary(&headers).unwrap().1,
            "5c02368e880e436dab70ed54e1c58209",
        );
    }

    fn create_stream() -> (
        mpsc::UnboundedSender<Result<Bytes, PayloadError>>,
        impl Stream<Item = Result<Bytes, PayloadError>>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();

        (
            tx,
            UnboundedReceiverStream::new(rx).map(|res| res.map_err(|_| panic!())),
        )
    }

    fn create_simple_request_with_header() -> (Bytes, HeaderMap) {
        let (body, headers) = crate::test::create_form_data_payload_and_headers_with_boundary(
            BOUNDARY,
            "file",
            Some("fn.txt".to_owned()),
            Some(mime::TEXT_PLAIN_UTF_8),
            Bytes::from_static(b"data"),
        );

        let mut buf = BytesMut::with_capacity(body.len() + 14);

        // add junk before form to test pre-boundary data rejection
        buf.put("testasdadsad\r\n".as_bytes());

        buf.put(body);

        (buf.freeze(), headers)
    }

    // TODO: use test utility when multi-file support is introduced
    fn create_double_request_with_header() -> (Bytes, HeaderMap) {
        let bytes = Bytes::from(
            "testasdadsad\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"fn.txt\"\r\n\
             Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
             test\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"fn.txt\"\r\n\
             Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
             data\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0--\r\n",
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(
                "multipart/mixed; boundary=\"abbc761f78ff4d7cb7573b5a23f96ef0\"",
            ),
        );
        (bytes, headers)
    }

    #[actix_rt::test]
    async fn test_multipart_no_end_crlf() {
        let (sender, payload) = create_stream();
        let (mut bytes, headers) = create_double_request_with_header();
        let bytes_stripped = bytes.split_to(bytes.len()); // strip crlf

        sender.send(Ok(bytes_stripped)).unwrap();
        drop(sender); // eof

        let mut multipart = Multipart::new(&headers, payload);

        match multipart.next().await.unwrap() {
            Ok(_) => {}
            _ => unreachable!(),
        }

        match multipart.next().await.unwrap() {
            Ok(_) => {}
            _ => unreachable!(),
        }

        match multipart.next().await {
            None => {}
            _ => unreachable!(),
        }
    }

    #[actix_rt::test]
    async fn test_multipart() {
        let (sender, payload) = create_stream();
        let (bytes, headers) = create_double_request_with_header();

        sender.send(Ok(bytes)).unwrap();

        let mut multipart = Multipart::new(&headers, payload);
        match multipart.next().await {
            Some(Ok(mut field)) => {
                let cd = field.content_disposition().unwrap();
                assert_eq!(cd.disposition, DispositionType::FormData);
                assert_eq!(cd.parameters[0], DispositionParam::Name("file".into()));

                assert_eq!(field.content_type().unwrap().type_(), mime::TEXT);
                assert_eq!(field.content_type().unwrap().subtype(), mime::PLAIN);

                match field.next().await.unwrap() {
                    Ok(chunk) => assert_eq!(chunk, "test"),
                    _ => unreachable!(),
                }
                match field.next().await {
                    None => {}
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }

        match multipart.next().await.unwrap() {
            Ok(mut field) => {
                assert_eq!(field.content_type().unwrap().type_(), mime::TEXT);
                assert_eq!(field.content_type().unwrap().subtype(), mime::PLAIN);

                match field.next().await {
                    Some(Ok(chunk)) => assert_eq!(chunk, "data"),
                    _ => unreachable!(),
                }
                match field.next().await {
                    None => {}
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }

        match multipart.next().await {
            None => {}
            _ => unreachable!(),
        }
    }

    // Loops, collecting all bytes until end-of-field
    async fn get_whole_field(field: &mut Field) -> BytesMut {
        let mut b = BytesMut::new();
        loop {
            match field.next().await {
                Some(Ok(chunk)) => b.extend_from_slice(&chunk),
                None => return b,
                _ => unreachable!(),
            }
        }
    }

    #[actix_rt::test]
    async fn test_stream() {
        let (bytes, headers) = create_double_request_with_header();
        let payload = stream::iter(bytes)
            .map(|byte| Ok(Bytes::copy_from_slice(&[byte])))
            .interleave_pending();

        let mut multipart = Multipart::new(&headers, payload);
        match multipart.next().await.unwrap() {
            Ok(mut field) => {
                let cd = field.content_disposition().unwrap();
                assert_eq!(cd.disposition, DispositionType::FormData);
                assert_eq!(cd.parameters[0], DispositionParam::Name("file".into()));

                assert_eq!(field.content_type().unwrap().type_(), mime::TEXT);
                assert_eq!(field.content_type().unwrap().subtype(), mime::PLAIN);

                assert_eq!(get_whole_field(&mut field).await, "test");
            }
            _ => unreachable!(),
        }

        match multipart.next().await {
            Some(Ok(mut field)) => {
                assert_eq!(field.content_type().unwrap().type_(), mime::TEXT);
                assert_eq!(field.content_type().unwrap().subtype(), mime::PLAIN);

                assert_eq!(get_whole_field(&mut field).await, "data");
            }
            _ => unreachable!(),
        }

        match multipart.next().await {
            None => {}
            _ => unreachable!(),
        }
    }

    #[actix_rt::test]
    async fn test_multipart_from_error() {
        let err = Error::ContentTypeMissing;
        let mut multipart = Multipart::from_error(err);
        assert!(multipart.next().await.unwrap().is_err())
    }

    #[actix_rt::test]
    async fn test_multipart_from_boundary() {
        let (_, payload) = create_stream();
        let (_, headers) = create_simple_request_with_header();
        let (ct, boundary) = Multipart::find_ct_and_boundary(&headers).unwrap();
        let _ = Multipart::from_ct_and_boundary(ct, boundary, payload);
    }

    #[actix_rt::test]
    async fn test_multipart_payload_consumption() {
        // with sample payload and HttpRequest with no headers
        let (_, inner_payload) = h1::Payload::create(false);
        let mut payload = actix_web::dev::Payload::from(inner_payload);
        let req = TestRequest::default().to_http_request();

        // multipart should generate an error
        let mut mp = Multipart::from_request(&req, &mut payload).await.unwrap();
        assert!(mp.next().await.unwrap().is_err());

        // and should not consume the payload
        match payload {
            actix_web::dev::Payload::H1 { .. } => {} //expected
            _ => unreachable!(),
        }
    }

    #[actix_rt::test]
    async fn no_content_disposition_form_data() {
        let bytes = Bytes::from(
            "testasdadsad\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Content-Length: 4\r\n\
             \r\n\
             test\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n",
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(
                "multipart/form-data; boundary=\"abbc761f78ff4d7cb7573b5a23f96ef0\"",
            ),
        );
        let payload = stream::iter(bytes)
            .map(|byte| Ok(Bytes::copy_from_slice(&[byte])))
            .interleave_pending();

        let mut multipart = Multipart::new(&headers, payload);
        let res = multipart.next().await.unwrap();
        assert_matches!(
            res.expect_err(
                "according to RFC 7578, form-data fields require a content-disposition header"
            ),
            Error::ContentDispositionMissing
        );
    }

    #[actix_rt::test]
    async fn no_content_disposition_non_form_data() {
        let bytes = Bytes::from(
            "testasdadsad\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Content-Length: 4\r\n\
             \r\n\
             test\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n",
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(
                "multipart/mixed; boundary=\"abbc761f78ff4d7cb7573b5a23f96ef0\"",
            ),
        );
        let payload = stream::iter(bytes)
            .map(|byte| Ok(Bytes::copy_from_slice(&[byte])))
            .interleave_pending();

        let mut multipart = Multipart::new(&headers, payload);
        let res = multipart.next().await.unwrap();
        res.unwrap();
    }

    #[actix_rt::test]
    async fn no_name_in_form_data_content_disposition() {
        let bytes = Bytes::from(
            "testasdadsad\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
             Content-Disposition: form-data; filename=\"fn.txt\"\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Content-Length: 4\r\n\
             \r\n\
             test\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n",
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(
                "multipart/form-data; boundary=\"abbc761f78ff4d7cb7573b5a23f96ef0\"",
            ),
        );
        let payload = stream::iter(bytes)
            .map(|byte| Ok(Bytes::copy_from_slice(&[byte])))
            .interleave_pending();

        let mut multipart = Multipart::new(&headers, payload);
        let res = multipart.next().await.unwrap();
        assert_matches!(
            res.expect_err("according to RFC 7578, form-data fields require a name attribute"),
            Error::ContentDispositionNameMissing
        );
    }

    #[actix_rt::test]
    async fn test_drop_multipart_dont_hang() {
        let (sender, payload) = create_stream();
        let (bytes, headers) = create_simple_request_with_header();
        sender.send(Ok(bytes)).unwrap();
        drop(sender); // eof

        let mut multipart = Multipart::new(&headers, payload);
        let mut field = multipart.next().await.unwrap().unwrap();

        drop(multipart);

        // should fail immediately
        match field.next().await {
            Some(Err(Error::NotConsumed)) => {}
            _ => panic!(),
        };
    }

    #[actix_rt::test]
    async fn test_drop_field_awaken_multipart() {
        let (sender, payload) = create_stream();
        let (bytes, headers) = create_double_request_with_header();
        sender.send(Ok(bytes)).unwrap();
        drop(sender); // eof

        let mut multipart = Multipart::new(&headers, payload);
        let mut field = multipart.next().await.unwrap().unwrap();

        let task = rt::spawn(async move {
            rt::time::sleep(Duration::from_millis(500)).await;
            assert_eq!(field.next().await.unwrap().unwrap(), "test");
            drop(field);
        });

        // dropping field should awaken current task
        let _ = multipart.next().await.unwrap().unwrap();
        task.await.unwrap();
    }
}
