//! Multipart response payload support.

use std::{
    cell::{Cell, RefCell, RefMut},
    cmp,
    convert::TryFrom,
    fmt,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_web::{
    error::{ParseError, PayloadError},
    http::header::{self, ContentDisposition, HeaderMap, HeaderName, HeaderValue},
};
use bytes::{Bytes, BytesMut};
use futures_core::stream::{LocalBoxStream, Stream};
use local_waker::LocalWaker;

use crate::error::MultipartError;

const MAX_HEADERS: usize = 32;

/// The server-side implementation of `multipart/form-data` requests.
///
/// This will parse the incoming stream into `MultipartItem` instances via its
/// Stream implementation.
/// `MultipartItem::Field` contains multipart field. `MultipartItem::Multipart`
/// is used for nested multipart streams.
pub struct Multipart {
    safety: Safety,
    error: Option<MultipartError>,
    inner: Option<InnerMultipart>,
}

enum InnerMultipartItem {
    None,
    Field(Rc<RefCell<InnerField>>),
}

#[derive(PartialEq, Debug)]
enum InnerState {
    /// Stream eof
    Eof,

    /// Skip data until first boundary
    FirstBoundary,

    /// Reading boundary
    Boundary,

    /// Reading Headers,
    Headers,
}

struct InnerMultipart {
    payload: PayloadRef,
    boundary: String,
    state: InnerState,
    item: InnerMultipartItem,
}

impl Multipart {
    /// Create multipart instance for boundary.
    pub fn new<S>(headers: &HeaderMap, stream: S) -> Multipart
    where
        S: Stream<Item = Result<Bytes, PayloadError>> + 'static,
    {
        match Self::boundary(headers) {
            Ok(boundary) => Multipart::from_boundary(boundary, stream),
            Err(err) => Multipart::from_error(err),
        }
    }

    /// Extract boundary info from headers.
    pub(crate) fn boundary(headers: &HeaderMap) -> Result<String, MultipartError> {
        headers
            .get(&header::CONTENT_TYPE)
            .ok_or(MultipartError::NoContentType)?
            .to_str()
            .ok()
            .and_then(|content_type| content_type.parse::<mime::Mime>().ok())
            .ok_or(MultipartError::ParseContentType)?
            .get_param(mime::BOUNDARY)
            .map(|boundary| boundary.as_str().to_owned())
            .ok_or(MultipartError::Boundary)
    }

    /// Create multipart instance for given boundary and stream
    pub(crate) fn from_boundary<S>(boundary: String, stream: S) -> Multipart
    where
        S: Stream<Item = Result<Bytes, PayloadError>> + 'static,
    {
        Multipart {
            error: None,
            safety: Safety::new(),
            inner: Some(InnerMultipart {
                boundary,
                payload: PayloadRef::new(PayloadBuffer::new(stream)),
                state: InnerState::FirstBoundary,
                item: InnerMultipartItem::None,
            }),
        }
    }

    /// Create Multipart instance from MultipartError
    pub(crate) fn from_error(err: MultipartError) -> Multipart {
        Multipart {
            error: Some(err),
            safety: Safety::new(),
            inner: None,
        }
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

impl InnerMultipart {
    fn read_headers(payload: &mut PayloadBuffer) -> Result<Option<HeaderMap>, MultipartError> {
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
                            let value = HeaderValue::try_from(h.value)
                                .map_err(|_| ParseError::Header)?;
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
                    Err(MultipartError::Boundary)
                } else if &chunk[boundary.len() + 2..] == b"\r\n" {
                    Ok(Some(false))
                } else if &chunk[boundary.len() + 2..boundary.len() + 4] == b"--"
                    && (chunk.len() == boundary.len() + 4
                        || &chunk[boundary.len() + 4..] == b"\r\n")
                {
                    Ok(Some(true))
                } else {
                    Err(MultipartError::Boundary)
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
                        return Err(MultipartError::Boundary);
                    }
                    if chunk.len() < boundary.len() {
                        continue;
                    }
                    if &chunk[..2] == b"--" && &chunk[2..chunk.len() - 2] == boundary.as_bytes()
                    {
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
        cx: &mut Context<'_>,
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
                                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
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

            let headers = if let Some(mut payload) = self.payload.get_mut(safety) {
                match self.state {
                    // read until first boundary
                    InnerState::FirstBoundary => {
                        match InnerMultipart::skip_until_boundary(
                            &mut *payload,
                            &self.boundary,
                        )? {
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
                        match InnerMultipart::read_boundary(&mut *payload, &self.boundary)? {
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
                    if let Some(headers) = InnerMultipart::read_headers(&mut *payload)? {
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

            // According to RFC 7578 §4.2, a Content-Disposition header must always be present and
            // set to "form-data".

            let content_disposition = headers
                .get(&header::CONTENT_DISPOSITION)
                .and_then(|cd| ContentDisposition::from_raw(cd).ok())
                .filter(|content_disposition| {
                    let is_form_data =
                        content_disposition.disposition == header::DispositionType::FormData;

                    let has_field_name = content_disposition
                        .parameters
                        .iter()
                        .any(|param| matches!(param, header::DispositionParam::Name(_)));

                    is_form_data && has_field_name
                });

            let cd = if let Some(content_disposition) = content_disposition {
                content_disposition
            } else {
                return Poll::Ready(Some(Err(MultipartError::NoContentDisposition)));
            };

            let ct: mime::Mime = headers
                .get(&header::CONTENT_TYPE)
                .and_then(|ct| ct.to_str().ok())
                .and_then(|ct| ct.parse().ok())
                .unwrap_or(mime::APPLICATION_OCTET_STREAM);

            self.state = InnerState::Boundary;

            // nested multipart stream is not supported
            if ct.type_() == mime::MULTIPART {
                return Poll::Ready(Some(Err(MultipartError::Nested)));
            }

            let field =
                InnerField::new_in_rc(self.payload.clone(), self.boundary.clone(), &headers)?;

            self.item = InnerMultipartItem::Field(Rc::clone(&field));

            Poll::Ready(Some(Ok(Field::new(
                safety.clone(cx),
                headers,
                ct,
                cd,
                field,
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

/// A single field in a multipart stream
pub struct Field {
    ct: mime::Mime,
    cd: ContentDisposition,
    headers: HeaderMap,
    inner: Rc<RefCell<InnerField>>,
    safety: Safety,
}

impl Field {
    fn new(
        safety: Safety,
        headers: HeaderMap,
        ct: mime::Mime,
        cd: ContentDisposition,
        inner: Rc<RefCell<InnerField>>,
    ) -> Self {
        Field {
            ct,
            cd,
            headers,
            inner,
            safety,
        }
    }

    /// Returns a reference to the field's header map.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns a reference to the field's content (mime) type.
    pub fn content_type(&self) -> &mime::Mime {
        &self.ct
    }

    /// Returns the field's Content-Disposition.
    ///
    /// Per [RFC 7578 §4.2]: "Each part MUST contain a Content-Disposition header field where the
    /// disposition type is `form-data`. The Content-Disposition header field MUST also contain an
    /// additional parameter of `name`; the value of the `name` parameter is the original field name
    /// from the form."
    ///
    /// This crate validates that it exists before returning a `Field`. As such, it is safe to
    /// unwrap `.content_disposition().get_name()`. The [name](Self::name) method is provided as
    /// a convenience.
    ///
    /// [RFC 7578 §4.2]: https://datatracker.ietf.org/doc/html/rfc7578#section-4.2
    pub fn content_disposition(&self) -> &ContentDisposition {
        &self.cd
    }

    /// Returns the field's name.
    ///
    /// See [content_disposition](Self::content_disposition) regarding guarantees about existence of
    /// the name field.
    pub fn name(&self) -> &str {
        self.content_disposition()
            .get_name()
            .expect("field name should be guaranteed to exist in multipart form-data")
    }
}

impl Stream for Field {
    type Item = Result<Bytes, MultipartError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let mut inner = this.inner.borrow_mut();
        if let Some(mut buffer) = inner.payload.as_ref().unwrap().get_mut(&this.safety) {
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
        writeln!(f, "\nField: {}", self.ct)?;
        writeln!(f, "  boundary: {}", self.inner.borrow().boundary)?;
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers.iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

struct InnerField {
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
            return if let Some(idx) = twoway::find_bytes(&payload.buf[pos..], b"\r") {
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

        let result = if let Some(mut payload) = self.payload.as_ref().unwrap().get_mut(s) {
            if !self.eof {
                let res = if let Some(ref mut len) = self.length {
                    InnerField::read_len(&mut *payload, len)
                } else {
                    InnerField::read_stream(&mut *payload, &self.boundary)
                };

                match res {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Some(Ok(bytes))) => return Poll::Ready(Some(Ok(bytes))),
                    Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                    Poll::Ready(None) => self.eof = true,
                }
            }

            match payload.readline() {
                Ok(None) => Poll::Pending,
                Ok(Some(line)) => {
                    if line.as_ref() != b"\r\n" {
                        log::warn!(
                            "multipart field did not read all the data or it is malformed"
                        );
                    }
                    Poll::Ready(None)
                }
                Err(e) => Poll::Ready(Some(Err(e))),
            }
        } else {
            Poll::Pending
        };

        if let Poll::Ready(None) = result {
            self.payload.take();
        }
        result
    }
}

struct PayloadRef {
    payload: Rc<RefCell<PayloadBuffer>>,
}

impl PayloadRef {
    fn new(payload: PayloadBuffer) -> PayloadRef {
        PayloadRef {
            payload: Rc::new(payload.into()),
        }
    }

    fn get_mut(&self, s: &Safety) -> Option<RefMut<'_, PayloadBuffer>> {
        if s.current() {
            Some(self.payload.borrow_mut())
        } else {
            None
        }
    }
}

impl Clone for PayloadRef {
    fn clone(&self) -> PayloadRef {
        PayloadRef {
            payload: Rc::clone(&self.payload),
        }
    }
}

/// Counter. It tracks of number of clones of payloads and give access to payload only to top most.
/// * When dropped, parent task is awakened. This is to support the case where Field is
/// dropped in a separate task than Multipart.
/// * Assumes that parent owners don't move to different tasks; only the top-most is allowed to.
/// * If dropped and is not top most owner, is_clean flag is set to false.
#[derive(Debug)]
struct Safety {
    task: LocalWaker,
    level: usize,
    payload: Rc<PhantomData<bool>>,
    clean: Rc<Cell<bool>>,
}

impl Safety {
    fn new() -> Safety {
        let payload = Rc::new(PhantomData);
        Safety {
            task: LocalWaker::new(),
            level: Rc::strong_count(&payload),
            clean: Rc::new(Cell::new(true)),
            payload,
        }
    }

    fn current(&self) -> bool {
        Rc::strong_count(&self.payload) == self.level && self.clean.get()
    }

    fn is_clean(&self) -> bool {
        self.clean.get()
    }

    fn clone(&self, cx: &mut Context<'_>) -> Safety {
        let payload = Rc::clone(&self.payload);
        let s = Safety {
            task: LocalWaker::new(),
            level: Rc::strong_count(&payload),
            clean: self.clean.clone(),
            payload,
        };
        s.task.register(cx.waker());
        s
    }
}

impl Drop for Safety {
    fn drop(&mut self) {
        if Rc::strong_count(&self.payload) != self.level {
            // Multipart dropped leaving a Field
            self.clean.set(false);
        }

        self.task.wake();
    }
}

/// Payload buffer.
struct PayloadBuffer {
    eof: bool,
    buf: BytesMut,
    stream: LocalBoxStream<'static, Result<Bytes, PayloadError>>,
}

impl PayloadBuffer {
    /// Constructs new `PayloadBuffer` instance.
    fn new<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, PayloadError>> + 'static,
    {
        PayloadBuffer {
            eof: false,
            buf: BytesMut::new(),
            stream: Box::pin(stream),
        }
    }

    fn poll_stream(&mut self, cx: &mut Context<'_>) -> Result<(), PayloadError> {
        loop {
            match Pin::new(&mut self.stream).poll_next(cx) {
                Poll::Ready(Some(Ok(data))) => self.buf.extend_from_slice(&data),
                Poll::Ready(Some(Err(e))) => return Err(e),
                Poll::Ready(None) => {
                    self.eof = true;
                    return Ok(());
                }
                Poll::Pending => return Ok(()),
            }
        }
    }

    /// Read exact number of bytes
    #[cfg(test)]
    fn read_exact(&mut self, size: usize) -> Option<Bytes> {
        if size <= self.buf.len() {
            Some(self.buf.split_to(size).freeze())
        } else {
            None
        }
    }

    fn read_max(&mut self, size: u64) -> Result<Option<Bytes>, MultipartError> {
        if !self.buf.is_empty() {
            let size = std::cmp::min(self.buf.len() as u64, size) as usize;
            Ok(Some(self.buf.split_to(size).freeze()))
        } else if self.eof {
            Err(MultipartError::Incomplete)
        } else {
            Ok(None)
        }
    }

    /// Read until specified ending
    fn read_until(&mut self, line: &[u8]) -> Result<Option<Bytes>, MultipartError> {
        let res = twoway::find_bytes(&self.buf, line)
            .map(|idx| self.buf.split_to(idx + line.len()).freeze());

        if res.is_none() && self.eof {
            Err(MultipartError::Incomplete)
        } else {
            Ok(res)
        }
    }

    /// Read bytes until new line delimiter
    fn readline(&mut self) -> Result<Option<Bytes>, MultipartError> {
        self.read_until(b"\n")
    }

    /// Read bytes until new line delimiter or eof
    fn readline_or_eof(&mut self) -> Result<Option<Bytes>, MultipartError> {
        match self.readline() {
            Err(MultipartError::Incomplete) if self.eof => Ok(Some(self.buf.split().freeze())),
            line => line,
        }
    }

    /// Put unprocessed data back to the buffer
    fn unprocessed(&mut self, data: Bytes) {
        let buf = BytesMut::from(data.as_ref());
        let buf = std::mem::replace(&mut self.buf, buf);
        self.buf.extend_from_slice(&buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use actix_http::h1::Payload;
    use actix_web::http::header::{DispositionParam, DispositionType};
    use actix_web::rt;
    use actix_web::test::TestRequest;
    use actix_web::FromRequest;
    use bytes::Bytes;
    use futures_util::{future::lazy, StreamExt};
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    #[actix_rt::test]
    async fn test_boundary() {
        let headers = HeaderMap::new();
        match Multipart::boundary(&headers) {
            Err(MultipartError::NoContentType) => {}
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("test"),
        );

        match Multipart::boundary(&headers) {
            Err(MultipartError::ParseContentType) => {}
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("multipart/mixed"),
        );
        match Multipart::boundary(&headers) {
            Err(MultipartError::Boundary) => {}
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
            Multipart::boundary(&headers).unwrap(),
            "5c02368e880e436dab70ed54e1c58209"
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
        let (mut bytes, headers) = create_simple_request_with_header();
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
        let (bytes, headers) = create_simple_request_with_header();

        sender.send(Ok(bytes)).unwrap();

        let mut multipart = Multipart::new(&headers, payload);
        match multipart.next().await {
            Some(Ok(mut field)) => {
                let cd = field.content_disposition();
                assert_eq!(cd.disposition, DispositionType::FormData);
                assert_eq!(cd.parameters[0], DispositionParam::Name("file".into()));

                assert_eq!(field.content_type().type_(), mime::TEXT);
                assert_eq!(field.content_type().subtype(), mime::PLAIN);

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
                assert_eq!(field.content_type().type_(), mime::TEXT);
                assert_eq!(field.content_type().subtype(), mime::PLAIN);

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
        let (bytes, headers) = create_simple_request_with_header();
        let payload = SlowStream::new(bytes);

        let mut multipart = Multipart::new(&headers, payload);
        match multipart.next().await.unwrap() {
            Ok(mut field) => {
                let cd = field.content_disposition();
                assert_eq!(cd.disposition, DispositionType::FormData);
                assert_eq!(cd.parameters[0], DispositionParam::Name("file".into()));

                assert_eq!(field.content_type().type_(), mime::TEXT);
                assert_eq!(field.content_type().subtype(), mime::PLAIN);

                assert_eq!(get_whole_field(&mut field).await, "test");
            }
            _ => unreachable!(),
        }

        match multipart.next().await {
            Some(Ok(mut field)) => {
                assert_eq!(field.content_type().type_(), mime::TEXT);
                assert_eq!(field.content_type().subtype(), mime::PLAIN);

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
        let (_, payload) = Payload::create(false);
        let mut payload = PayloadBuffer::new(payload);

        assert_eq!(payload.buf.len(), 0);
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        assert_eq!(None, payload.read_max(1).unwrap());
    }

    #[actix_rt::test]
    async fn test_eof() {
        let (mut sender, payload) = Payload::create(false);
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
        let (mut sender, payload) = Payload::create(false);
        let mut payload = PayloadBuffer::new(payload);
        assert_eq!(None, payload.read_max(1).unwrap());
        sender.set_error(PayloadError::Incomplete(None));
        lazy(|cx| payload.poll_stream(cx)).await.err().unwrap();
    }

    #[actix_rt::test]
    async fn test_readmax() {
        let (mut sender, payload) = Payload::create(false);
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
        let (mut sender, payload) = Payload::create(false);
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
        let (mut sender, payload) = Payload::create(false);
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
        let err = MultipartError::NoContentType;
        let mut multipart = Multipart::from_error(err);
        assert!(multipart.next().await.unwrap().is_err())
    }

    #[actix_rt::test]
    async fn test_multipart_from_boundary() {
        let (_, payload) = create_stream();
        let (_, headers) = create_simple_request_with_header();
        let boundary = Multipart::boundary(&headers);
        assert!(boundary.is_ok());
        let _ = Multipart::from_boundary(boundary.unwrap(), payload);
    }

    #[actix_rt::test]
    async fn test_multipart_payload_consumption() {
        // with sample payload and HttpRequest with no headers
        let (_, inner_payload) = Payload::create(false);
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
    async fn no_content_disposition() {
        let bytes = Bytes::from(
            "testasdadsad\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
             Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
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
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            MultipartError::NoContentDisposition,
        ));
    }

    #[actix_rt::test]
    async fn no_name_in_content_disposition() {
        let bytes = Bytes::from(
            "testasdadsad\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
             Content-Disposition: form-data; filename=\"fn.txt\"\r\n\
             Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
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
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            MultipartError::NoContentDisposition,
        ));
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
        let (bytes, headers) = create_simple_request_with_header();
        sender.send(Ok(bytes)).unwrap();
        drop(sender); // eof

        let mut multipart = Multipart::new(&headers, payload);
        let mut field = multipart.next().await.unwrap().unwrap();

        let task = rt::spawn(async move {
            rt::time::sleep(Duration::from_secs(1)).await;
            assert_eq!(field.next().await.unwrap().unwrap(), "test");
            drop(field);
        });

        // dropping field should awaken current task
        let _ = multipart.next().await.unwrap().unwrap();
        task.await.unwrap();
    }
}
