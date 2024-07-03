//! Multipart response payload support.

use std::{
    cell::RefCell,
    cmp, fmt,
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
    error::MultipartError,
    payload::{PayloadBuffer, PayloadRef},
    safety::Safety,
};

const MAX_HEADERS: usize = 32;

/// The server-side implementation of `multipart/form-data` requests.
///
/// This will parse the incoming stream into `MultipartItem` instances via its `Stream`
/// implementation. `MultipartItem::Field` contains multipart field. `MultipartItem::Multipart` is
/// used for nested multipart streams.
pub struct Multipart {
    safety: Safety,
    inner: Option<InnerMultipart>,
    error: Option<MultipartError>,
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
    pub(crate) fn find_ct_and_boundary(
        headers: &HeaderMap,
    ) -> Result<(Mime, String), MultipartError> {
        let content_type = headers
            .get(&header::CONTENT_TYPE)
            .ok_or(MultipartError::ContentTypeMissing)?
            .to_str()
            .ok()
            .and_then(|content_type| content_type.parse::<Mime>().ok())
            .ok_or(MultipartError::ContentTypeParse)?;

        if content_type.type_() != mime::MULTIPART {
            return Err(MultipartError::ContentTypeIncompatible);
        }

        let boundary = content_type
            .get_param(mime::BOUNDARY)
            .ok_or(MultipartError::BoundaryMissing)?
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
            inner: Some(InnerMultipart {
                payload: PayloadRef::new(PayloadBuffer::new(stream)),
                content_type: ct,
                boundary,
                state: InnerState::FirstBoundary,
                item: InnerMultipartItem::None,
            }),
            error: None,
        }
    }

    /// Constructs a new multipart reader from given `MultipartError`.
    pub(crate) fn from_error(err: MultipartError) -> Multipart {
        Multipart {
            error: Some(err),
            safety: Safety::new(),
            inner: None,
        }
    }

    /// Return requests parsed Content-Type or raise the stored error.
    pub(crate) fn content_type_or_bail(&mut self) -> Result<mime::Mime, MultipartError> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }

        Ok(self
            .inner
            .as_ref()
            // TODO: look into using enum instead of two options
            .expect("multipart requests should have state")
            .content_type
            .clone())
    }
}

impl Stream for Multipart {
    type Item = Result<Field, MultipartError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        match this.inner.as_mut() {
            Some(inner) => {
                if let Some(mut buffer) = inner.payload.get_mut(&this.safety) {
                    // check safety and poll read payload to buffer.
                    buffer.poll_stream(cx)?;
                } else if !this.safety.is_clean() {
                    // safety violation
                    return Poll::Ready(Some(Err(MultipartError::NotConsumed)));
                } else {
                    return Poll::Pending;
                }

                inner.poll(&this.safety, cx)
            }
            None => Poll::Ready(Some(Err(this
                .error
                .take()
                .expect("Multipart polled after finish")))),
        }
    }
}

#[derive(PartialEq, Debug)]
enum InnerState {
    /// Stream EOF.
    Eof,

    /// Skip data until first boundary.
    FirstBoundary,

    /// Reading boundary.
    Boundary,

    /// Reading Headers.
    Headers,
}

enum InnerMultipartItem {
    None,
    Field(Rc<RefCell<InnerField>>),
}

struct InnerMultipart {
    /// Request's payload stream & buffer.
    payload: PayloadRef,

    /// Request's Content-Type.
    ///
    /// Guaranteed to have "multipart" top-level media type, i.e., `multipart/*`.
    content_type: Mime,

    /// Field boundary.
    boundary: String,

    state: InnerState,
    item: InnerMultipartItem,
}

impl InnerMultipart {
    fn read_field_headers(
        payload: &mut PayloadBuffer,
    ) -> Result<Option<HeaderMap>, MultipartError> {
        match payload.read_until(b"\r\n\r\n")? {
            None => {
                if payload.eof {
                    Err(MultipartError::Incomplete)
                } else {
                    Ok(None)
                }
            }
            Some(bytes) => {
                let mut hdrs = [httparse::EMPTY_HEADER; MAX_HEADERS];

                match httparse::parse_headers(&bytes, &mut hdrs) {
                    Ok(httparse::Status::Complete((_, hdrs))) => {
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
                    Ok(httparse::Status::Partial) => Err(ParseError::Header.into()),
                    Err(err) => Err(ParseError::from(err).into()),
                }
            }
        }
    }

    fn read_boundary(
        payload: &mut PayloadBuffer,
        boundary: &str,
    ) -> Result<Option<bool>, MultipartError> {
        // TODO: need to read epilogue
        match payload.readline_or_eof()? {
            None => {
                if payload.eof {
                    Ok(Some(true))
                } else {
                    Ok(None)
                }
            }
            Some(chunk) => {
                if chunk.len() < boundary.len() + 4
                    || &chunk[..2] != b"--"
                    || &chunk[2..boundary.len() + 2] != boundary.as_bytes()
                {
                    Err(MultipartError::BoundaryMissing)
                } else if &chunk[boundary.len() + 2..] == b"\r\n" {
                    Ok(Some(false))
                } else if &chunk[boundary.len() + 2..boundary.len() + 4] == b"--"
                    && (chunk.len() == boundary.len() + 4
                        || &chunk[boundary.len() + 4..] == b"\r\n")
                {
                    Ok(Some(true))
                } else {
                    Err(MultipartError::BoundaryMissing)
                }
            }
        }
    }

    fn skip_until_boundary(
        payload: &mut PayloadBuffer,
        boundary: &str,
    ) -> Result<Option<bool>, MultipartError> {
        let mut eof = false;
        loop {
            match payload.readline()? {
                Some(chunk) => {
                    if chunk.is_empty() {
                        return Err(MultipartError::BoundaryMissing);
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
                        Err(MultipartError::Incomplete)
                    } else {
                        Ok(None)
                    };
                }
            }
        }
        Ok(Some(eof))
    }

    fn poll(
        &mut self,
        safety: &Safety,
        cx: &Context<'_>,
    ) -> Poll<Option<Result<Field, MultipartError>>> {
        if self.state == InnerState::Eof {
            Poll::Ready(None)
        } else {
            // release field
            loop {
                // Nested multipart streams of fields has to be consumed
                // before switching to next
                if safety.current() {
                    let stop = match self.item {
                        InnerMultipartItem::Field(ref mut field) => {
                            match field.borrow_mut().poll(safety) {
                                Poll::Pending => return Poll::Pending,
                                Poll::Ready(Some(Ok(_))) => continue,
                                Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err))),
                                Poll::Ready(None) => true,
                            }
                        }
                        InnerMultipartItem::None => false,
                    };
                    if stop {
                        self.item = InnerMultipartItem::None;
                    }
                    if let InnerMultipartItem::None = self.item {
                        break;
                    }
                }
            }

            let field_headers = if let Some(mut payload) = self.payload.get_mut(safety) {
                match self.state {
                    // read until first boundary
                    InnerState::FirstBoundary => {
                        match InnerMultipart::skip_until_boundary(&mut payload, &self.boundary)? {
                            Some(eof) => {
                                if eof {
                                    self.state = InnerState::Eof;
                                    return Poll::Ready(None);
                                } else {
                                    self.state = InnerState::Headers;
                                }
                            }
                            None => return Poll::Pending,
                        }
                    }
                    // read boundary
                    InnerState::Boundary => {
                        match InnerMultipart::read_boundary(&mut payload, &self.boundary)? {
                            None => return Poll::Pending,
                            Some(eof) => {
                                if eof {
                                    self.state = InnerState::Eof;
                                    return Poll::Ready(None);
                                } else {
                                    self.state = InnerState::Headers;
                                }
                            }
                        }
                    }
                    _ => {}
                }

                // read field headers for next field
                if self.state == InnerState::Headers {
                    if let Some(headers) = InnerMultipart::read_field_headers(&mut payload)? {
                        self.state = InnerState::Boundary;
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
                    return Poll::Ready(Some(Err(MultipartError::ContentDispositionMissing)));
                };

                let Some(field_name) = cd.get_name() else {
                    return Poll::Ready(Some(Err(MultipartError::ContentDispositionNameMissing)));
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

            self.state = InnerState::Boundary;

            // nested multipart stream is not supported
            if let Some(mime) = &field_content_type {
                if mime.type_() == mime::MULTIPART {
                    return Poll::Ready(Some(Err(MultipartError::Nested)));
                }
            }

            let field_inner =
                InnerField::new_in_rc(self.payload.clone(), self.boundary.clone(), &field_headers)?;

            self.item = InnerMultipartItem::Field(Rc::clone(&field_inner));

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

impl Drop for InnerMultipart {
    fn drop(&mut self) {
        // InnerMultipartItem::Field has to be dropped first because of Safety.
        self.item = InnerMultipartItem::None;
    }
}

/// A single field in a multipart stream.
pub struct Field {
    /// Field's Content-Type.
    content_type: Option<Mime>,

    /// Field's Content-Disposition.
    content_disposition: Option<ContentDisposition>,

    /// Form field name.
    ///
    /// A non-optional storage for form field names to avoid unwraps in `form` module. Will be an
    /// empty string in non-form contexts.
    ///
    // INVARIANT: always non-empty when request content-type is multipart/form-data.
    pub(crate) form_field_name: String,

    /// Field's header map.
    headers: HeaderMap,

    safety: Safety,
    inner: Rc<RefCell<InnerField>>,
}

impl Field {
    fn new(
        content_type: Option<Mime>,
        content_disposition: Option<ContentDisposition>,
        form_field_name: Option<String>,
        headers: HeaderMap,
        safety: Safety,
        inner: Rc<RefCell<InnerField>>,
    ) -> Self {
        Field {
            content_type,
            content_disposition,
            form_field_name: form_field_name.unwrap_or_default(),
            headers,
            inner,
            safety,
        }
    }

    /// Returns a reference to the field's header map.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns a reference to the field's content (mime) type, if it is supplied by the client.
    ///
    /// According to [RFC 7578](https://www.rfc-editor.org/rfc/rfc7578#section-4.4), if it is not
    /// present, it should default to "text/plain". Note it is the responsibility of the client to
    /// provide the appropriate content type, there is no attempt to validate this by the server.
    pub fn content_type(&self) -> Option<&Mime> {
        self.content_type.as_ref()
    }

    /// Returns this field's parsed Content-Disposition header, if set.
    ///
    /// # Validation
    ///
    /// Per [RFC 7578 §4.2], the parts of a multipart/form-data payload MUST contain a
    /// Content-Disposition header field where the disposition type is `form-data` and MUST also
    /// contain an additional parameter of `name` with its value being the original field name from
    /// the form. This requirement is enforced during extraction for multipart/form-data requests,
    /// but not other kinds of multipart requests (such as multipart/related).
    ///
    /// As such, it is safe to `.unwrap()` calls `.content_disposition()` if you've verified.
    ///
    /// The [`name()`](Self::name) method is also provided as a convenience for obtaining the
    /// aforementioned name parameter.
    ///
    /// [RFC 7578 §4.2]: https://datatracker.ietf.org/doc/html/rfc7578#section-4.2
    pub fn content_disposition(&self) -> Option<&ContentDisposition> {
        self.content_disposition.as_ref()
    }

    /// Returns the field's name, if set.
    ///
    /// See [`content_disposition()`](Self::content_disposition) regarding guarantees on presence of
    /// the "name" field.
    pub fn name(&self) -> Option<&str> {
        self.content_disposition()?.get_name()
    }
}

impl Stream for Field {
    type Item = Result<Bytes, MultipartError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let mut inner = this.inner.borrow_mut();

        if let Some(mut buffer) = inner
            .payload
            .as_ref()
            .expect("Field should not be polled after completion")
            .get_mut(&this.safety)
        {
            // check safety and poll read payload to buffer.
            buffer.poll_stream(cx)?;
        } else if !this.safety.is_clean() {
            // safety violation
            return Poll::Ready(Some(Err(MultipartError::NotConsumed)));
        } else {
            return Poll::Pending;
        }

        inner.poll(&this.safety)
    }
}

impl fmt::Debug for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ct) = &self.content_type {
            writeln!(f, "\nField: {}", ct)?;
        } else {
            writeln!(f, "\nField:")?;
        }
        writeln!(f, "  boundary: {}", self.inner.borrow().boundary)?;
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers.iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

struct InnerField {
    /// Payload is initialized as Some and is `take`n when the field stream finishes.
    payload: Option<PayloadRef>,
    boundary: String,
    eof: bool,
    length: Option<u64>,
}

impl InnerField {
    fn new_in_rc(
        payload: PayloadRef,
        boundary: String,
        headers: &HeaderMap,
    ) -> Result<Rc<RefCell<InnerField>>, PayloadError> {
        Self::new(payload, boundary, headers).map(|this| Rc::new(RefCell::new(this)))
    }

    fn new(
        payload: PayloadRef,
        boundary: String,
        headers: &HeaderMap,
    ) -> Result<InnerField, PayloadError> {
        let len = if let Some(len) = headers.get(&header::CONTENT_LENGTH) {
            match len.to_str().ok().and_then(|len| len.parse::<u64>().ok()) {
                Some(len) => Some(len),
                None => return Err(PayloadError::Incomplete(None)),
            }
        } else {
            None
        };

        Ok(InnerField {
            boundary,
            payload: Some(payload),
            eof: false,
            length: len,
        })
    }

    /// Reads body part content chunk of the specified size.
    /// The body part must has `Content-Length` header with proper value.
    fn read_len(
        payload: &mut PayloadBuffer,
        size: &mut u64,
    ) -> Poll<Option<Result<Bytes, MultipartError>>> {
        if *size == 0 {
            Poll::Ready(None)
        } else {
            match payload.read_max(*size)? {
                Some(mut chunk) => {
                    let len = cmp::min(chunk.len() as u64, *size);
                    *size -= len;
                    let ch = chunk.split_to(len as usize);
                    if !chunk.is_empty() {
                        payload.unprocessed(chunk);
                    }
                    Poll::Ready(Some(Ok(ch)))
                }
                None => {
                    if payload.eof && (*size != 0) {
                        Poll::Ready(Some(Err(MultipartError::Incomplete)))
                    } else {
                        Poll::Pending
                    }
                }
            }
        }
    }

    /// Reads content chunk of body part with unknown length.
    ///
    /// The `Content-Length` header for body part is not necessary.
    fn read_stream(
        payload: &mut PayloadBuffer,
        boundary: &str,
    ) -> Poll<Option<Result<Bytes, MultipartError>>> {
        let mut pos = 0;

        let len = payload.buf.len();
        if len == 0 {
            return if payload.eof {
                Poll::Ready(Some(Err(MultipartError::Incomplete)))
            } else {
                Poll::Pending
            };
        }

        // check boundary
        if len > 4 && payload.buf[0] == b'\r' {
            let b_len = if &payload.buf[..2] == b"\r\n" && &payload.buf[2..4] == b"--" {
                Some(4)
            } else if &payload.buf[1..3] == b"--" {
                Some(3)
            } else {
                None
            };

            if let Some(b_len) = b_len {
                let b_size = boundary.len() + b_len;
                if len < b_size {
                    return Poll::Pending;
                } else if &payload.buf[b_len..b_size] == boundary.as_bytes() {
                    // found boundary
                    return Poll::Ready(None);
                }
            }
        }

        loop {
            return if let Some(idx) = memchr::memmem::find(&payload.buf[pos..], b"\r") {
                let cur = pos + idx;

                // check if we have enough data for boundary detection
                if cur + 4 > len {
                    if cur > 0 {
                        Poll::Ready(Some(Ok(payload.buf.split_to(cur).freeze())))
                    } else {
                        Poll::Pending
                    }
                } else {
                    // check boundary
                    if (&payload.buf[cur..cur + 2] == b"\r\n"
                        && &payload.buf[cur + 2..cur + 4] == b"--")
                        || (&payload.buf[cur..=cur] == b"\r"
                            && &payload.buf[cur + 1..cur + 3] == b"--")
                    {
                        if cur != 0 {
                            // return buffer
                            Poll::Ready(Some(Ok(payload.buf.split_to(cur).freeze())))
                        } else {
                            pos = cur + 1;
                            continue;
                        }
                    } else {
                        // not boundary
                        pos = cur + 1;
                        continue;
                    }
                }
            } else {
                Poll::Ready(Some(Ok(payload.buf.split().freeze())))
            };
        }
    }

    fn poll(&mut self, s: &Safety) -> Poll<Option<Result<Bytes, MultipartError>>> {
        if self.payload.is_none() {
            return Poll::Ready(None);
        }

        let result = if let Some(mut payload) = self
            .payload
            .as_ref()
            .expect("Field should not be polled after completion")
            .get_mut(s)
        {
            if !self.eof {
                let res = if let Some(ref mut len) = self.length {
                    InnerField::read_len(&mut payload, len)
                } else {
                    InnerField::read_stream(&mut payload, &self.boundary)
                };

                match res {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Some(Ok(bytes))) => return Poll::Ready(Some(Ok(bytes))),
                    Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err))),
                    Poll::Ready(None) => self.eof = true,
                }
            }

            match payload.readline() {
                Ok(None) => Poll::Pending,
                Ok(Some(line)) => {
                    if line.as_ref() != b"\r\n" {
                        log::warn!("multipart field did not read all the data or it is malformed");
                    }
                    Poll::Ready(None)
                }
                Err(err) => Poll::Ready(Some(Err(err))),
            }
        } else {
            Poll::Pending
        };

        if let Poll::Ready(None) = result {
            // drop payload buffer and make future un-poll-able
            let _ = self.payload.take();
        }

        result
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
    use futures_util::{future::lazy, StreamExt as _};
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    use super::*;

    const BOUNDARY: &str = "abbc761f78ff4d7cb7573b5a23f96ef0";

    #[actix_rt::test]
    async fn test_boundary() {
        let headers = HeaderMap::new();
        match Multipart::find_ct_and_boundary(&headers) {
            Err(MultipartError::ContentTypeMissing) => {}
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("test"),
        );

        match Multipart::find_ct_and_boundary(&headers) {
            Err(MultipartError::ContentTypeParse) => {}
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("multipart/mixed"),
        );
        match Multipart::find_ct_and_boundary(&headers) {
            Err(MultipartError::BoundaryMissing) => {}
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

    // Stream that returns from a Bytes, one char at a time and Pending every other poll()
    struct SlowStream {
        bytes: Bytes,
        pos: usize,
        ready: bool,
    }

    impl SlowStream {
        fn new(bytes: Bytes) -> SlowStream {
            SlowStream {
                bytes,
                pos: 0,
                ready: false,
            }
        }
    }

    impl Stream for SlowStream {
        type Item = Result<Bytes, PayloadError>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let this = self.get_mut();
            if !this.ready {
                this.ready = true;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }

            if this.pos == this.bytes.len() {
                return Poll::Ready(None);
            }

            let res = Poll::Ready(Some(Ok(this.bytes.slice(this.pos..(this.pos + 1)))));
            this.pos += 1;
            this.ready = false;
            res
        }
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
        let payload = SlowStream::new(bytes);

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
    async fn test_basic() {
        let (_, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new(payload);

        assert_eq!(payload.buf.len(), 0);
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        assert_eq!(None, payload.read_max(1).unwrap());
    }

    #[actix_rt::test]
    async fn test_eof() {
        let (mut sender, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new(payload);

        assert_eq!(None, payload.read_max(4).unwrap());
        sender.feed_data(Bytes::from("data"));
        sender.feed_eof();
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();

        assert_eq!(Some(Bytes::from("data")), payload.read_max(4).unwrap());
        assert_eq!(payload.buf.len(), 0);
        assert!(payload.read_max(1).is_err());
        assert!(payload.eof);
    }

    #[actix_rt::test]
    async fn test_err() {
        let (mut sender, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new(payload);
        assert_eq!(None, payload.read_max(1).unwrap());
        sender.set_error(PayloadError::Incomplete(None));
        lazy(|cx| payload.poll_stream(cx)).await.err().unwrap();
    }

    #[actix_rt::test]
    async fn test_readmax() {
        let (mut sender, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new(payload);

        sender.feed_data(Bytes::from("line1"));
        sender.feed_data(Bytes::from("line2"));
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        assert_eq!(payload.buf.len(), 10);

        assert_eq!(Some(Bytes::from("line1")), payload.read_max(5).unwrap());
        assert_eq!(payload.buf.len(), 5);

        assert_eq!(Some(Bytes::from("line2")), payload.read_max(5).unwrap());
        assert_eq!(payload.buf.len(), 0);
    }

    #[actix_rt::test]
    async fn test_readexactly() {
        let (mut sender, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new(payload);

        assert_eq!(None, payload.read_exact(2));

        sender.feed_data(Bytes::from("line1"));
        sender.feed_data(Bytes::from("line2"));
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();

        assert_eq!(Some(Bytes::from_static(b"li")), payload.read_exact(2));
        assert_eq!(payload.buf.len(), 8);

        assert_eq!(Some(Bytes::from_static(b"ne1l")), payload.read_exact(4));
        assert_eq!(payload.buf.len(), 4);
    }

    #[actix_rt::test]
    async fn test_readuntil() {
        let (mut sender, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new(payload);

        assert_eq!(None, payload.read_until(b"ne").unwrap());

        sender.feed_data(Bytes::from("line1"));
        sender.feed_data(Bytes::from("line2"));
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();

        assert_eq!(
            Some(Bytes::from("line")),
            payload.read_until(b"ne").unwrap()
        );
        assert_eq!(payload.buf.len(), 6);

        assert_eq!(
            Some(Bytes::from("1line2")),
            payload.read_until(b"2").unwrap()
        );
        assert_eq!(payload.buf.len(), 0);
    }

    #[actix_rt::test]
    async fn test_multipart_from_error() {
        let err = MultipartError::ContentTypeMissing;
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
        let payload = SlowStream::new(bytes);

        let mut multipart = Multipart::new(&headers, payload);
        let res = multipart.next().await.unwrap();
        assert_matches!(
            res.expect_err(
                "according to RFC 7578, form-data fields require a content-disposition header"
            ),
            MultipartError::ContentDispositionMissing
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
        let payload = SlowStream::new(bytes);

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
        let payload = SlowStream::new(bytes);

        let mut multipart = Multipart::new(&headers, payload);
        let res = multipart.next().await.unwrap();
        assert_matches!(
            res.expect_err("according to RFC 7578, form-data fields require a name attribute"),
            MultipartError::ContentDispositionNameMissing
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
            Some(Err(MultipartError::NotConsumed)) => {}
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
