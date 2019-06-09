//! Multipart payload support
use std::cell::{Cell, RefCell, UnsafeCell};
use std::marker::PhantomData;
use std::rc::Rc;
use std::{cmp, fmt};

use bytes::{Bytes, BytesMut};
use futures::task::{current as current_task, Task};
use futures::{Async, Poll, Stream};
use httparse;
use mime;

use actix_web::error::{ParseError, PayloadError};
use actix_web::http::header::{
    self, ContentDisposition, HeaderMap, HeaderName, HeaderValue,
};
use actix_web::http::HttpTryFrom;

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
    inner: Option<Rc<RefCell<InnerMultipart>>>,
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
        S: Stream<Item = Bytes, Error = PayloadError> + 'static,
    {
        match Self::boundary(headers) {
            Ok(boundary) => Multipart {
                error: None,
                safety: Safety::new(),
                inner: Some(Rc::new(RefCell::new(InnerMultipart {
                    boundary,
                    payload: PayloadRef::new(PayloadBuffer::new(Box::new(stream))),
                    state: InnerState::FirstBoundary,
                    item: InnerMultipartItem::None,
                }))),
            },
            Err(err) => Multipart {
                error: Some(err),
                safety: Safety::new(),
                inner: None,
            },
        }
    }

    /// Extract boundary info from headers.
    fn boundary(headers: &HeaderMap) -> Result<String, MultipartError> {
        if let Some(content_type) = headers.get(&header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                if let Ok(ct) = content_type.parse::<mime::Mime>() {
                    if let Some(boundary) = ct.get_param(mime::BOUNDARY) {
                        Ok(boundary.as_str().to_owned())
                    } else {
                        Err(MultipartError::Boundary)
                    }
                } else {
                    Err(MultipartError::ParseContentType)
                }
            } else {
                Err(MultipartError::ParseContentType)
            }
        } else {
            Err(MultipartError::NoContentType)
        }
    }
}

impl Stream for Multipart {
    type Item = Field;
    type Error = MultipartError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some(err) = self.error.take() {
            Err(err)
        } else if self.safety.current() {
            let mut inner = self.inner.as_mut().unwrap().borrow_mut();
            if let Some(payload) = inner.payload.get_mut(&self.safety) {
                payload.poll_stream()?;
            }
            inner.poll(&self.safety)
        } else if !self.safety.is_clean() {
            Err(MultipartError::NotConsumed)
        } else {
            Ok(Async::NotReady)
        }
    }
}

impl InnerMultipart {
    fn read_headers(
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
                            if let Ok(name) = HeaderName::try_from(h.name) {
                                if let Ok(value) = HeaderValue::try_from(h.value) {
                                    headers.append(name, value);
                                } else {
                                    return Err(ParseError::Header.into());
                                }
                            } else {
                                return Err(ParseError::Header.into());
                            }
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
        match payload.readline()? {
            None => {
                if payload.eof {
                    Ok(Some(true))
                } else {
                    Ok(None)
                }
            }
            Some(chunk) => {
                if chunk.len() == boundary.len() + 4
                    && &chunk[..2] == b"--"
                    && &chunk[2..boundary.len() + 2] == boundary.as_bytes()
                {
                    Ok(Some(false))
                } else if chunk.len() == boundary.len() + 6
                    && &chunk[..2] == b"--"
                    && &chunk[2..boundary.len() + 2] == boundary.as_bytes()
                    && &chunk[boundary.len() + 2..boundary.len() + 4] == b"--"
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
                    if &chunk[..2] == b"--"
                        && &chunk[2..chunk.len() - 2] == boundary.as_bytes()
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

    fn poll(&mut self, safety: &Safety) -> Poll<Option<Field>, MultipartError> {
        if self.state == InnerState::Eof {
            Ok(Async::Ready(None))
        } else {
            // release field
            loop {
                // Nested multipart streams of fields has to be consumed
                // before switching to next
                if safety.current() {
                    let stop = match self.item {
                        InnerMultipartItem::Field(ref mut field) => {
                            match field.borrow_mut().poll(safety)? {
                                Async::NotReady => return Ok(Async::NotReady),
                                Async::Ready(Some(_)) => continue,
                                Async::Ready(None) => true,
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

            let headers = if let Some(payload) = self.payload.get_mut(safety) {
                match self.state {
                    // read until first boundary
                    InnerState::FirstBoundary => {
                        match InnerMultipart::skip_until_boundary(
                            payload,
                            &self.boundary,
                        )? {
                            Some(eof) => {
                                if eof {
                                    self.state = InnerState::Eof;
                                    return Ok(Async::Ready(None));
                                } else {
                                    self.state = InnerState::Headers;
                                }
                            }
                            None => return Ok(Async::NotReady),
                        }
                    }
                    // read boundary
                    InnerState::Boundary => {
                        match InnerMultipart::read_boundary(payload, &self.boundary)? {
                            None => return Ok(Async::NotReady),
                            Some(eof) => {
                                if eof {
                                    self.state = InnerState::Eof;
                                    return Ok(Async::Ready(None));
                                } else {
                                    self.state = InnerState::Headers;
                                }
                            }
                        }
                    }
                    _ => (),
                }

                // read field headers for next field
                if self.state == InnerState::Headers {
                    if let Some(headers) = InnerMultipart::read_headers(payload)? {
                        self.state = InnerState::Boundary;
                        headers
                    } else {
                        return Ok(Async::NotReady);
                    }
                } else {
                    unreachable!()
                }
            } else {
                log::debug!("NotReady: field is in flight");
                return Ok(Async::NotReady);
            };

            // content type
            let mut mt = mime::APPLICATION_OCTET_STREAM;
            if let Some(content_type) = headers.get(&header::CONTENT_TYPE) {
                if let Ok(content_type) = content_type.to_str() {
                    if let Ok(ct) = content_type.parse::<mime::Mime>() {
                        mt = ct;
                    }
                }
            }

            self.state = InnerState::Boundary;

            // nested multipart stream
            if mt.type_() == mime::MULTIPART {
                Err(MultipartError::Nested)
            } else {
                let field = Rc::new(RefCell::new(InnerField::new(
                    self.payload.clone(),
                    self.boundary.clone(),
                    &headers,
                )?));
                self.item = InnerMultipartItem::Field(Rc::clone(&field));

                Ok(Async::Ready(Some(Field::new(
                    safety.clone(),
                    headers,
                    mt,
                    field,
                ))))
            }
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
    headers: HeaderMap,
    inner: Rc<RefCell<InnerField>>,
    safety: Safety,
}

impl Field {
    fn new(
        safety: Safety,
        headers: HeaderMap,
        ct: mime::Mime,
        inner: Rc<RefCell<InnerField>>,
    ) -> Self {
        Field {
            ct,
            headers,
            inner,
            safety,
        }
    }

    /// Get a map of headers
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get the content type of the field
    pub fn content_type(&self) -> &mime::Mime {
        &self.ct
    }

    /// Get the content disposition of the field, if it exists
    pub fn content_disposition(&self) -> Option<ContentDisposition> {
        // RFC 7578: 'Each part MUST contain a Content-Disposition header field
        // where the disposition type is "form-data".'
        if let Some(content_disposition) = self.headers.get(&header::CONTENT_DISPOSITION)
        {
            ContentDisposition::from_raw(content_disposition).ok()
        } else {
            None
        }
    }
}

impl Stream for Field {
    type Item = Bytes;
    type Error = MultipartError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if self.safety.current() {
            let mut inner = self.inner.borrow_mut();
            if let Some(payload) = inner.payload.as_ref().unwrap().get_mut(&self.safety)
            {
                payload.poll_stream()?;
            }

            inner.poll(&self.safety)
        } else if !self.safety.is_clean() {
            return Err(MultipartError::NotConsumed);
        } else {
            Ok(Async::NotReady)
        }
    }
}

impl fmt::Debug for Field {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
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
    fn new(
        payload: PayloadRef,
        boundary: String,
        headers: &HeaderMap,
    ) -> Result<InnerField, PayloadError> {
        let len = if let Some(len) = headers.get(&header::CONTENT_LENGTH) {
            if let Ok(s) = len.to_str() {
                if let Ok(len) = s.parse::<u64>() {
                    Some(len)
                } else {
                    return Err(PayloadError::Incomplete(None));
                }
            } else {
                return Err(PayloadError::Incomplete(None));
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
    ) -> Poll<Option<Bytes>, MultipartError> {
        if *size == 0 {
            Ok(Async::Ready(None))
        } else {
            match payload.read_max(*size)? {
                Some(mut chunk) => {
                    let len = cmp::min(chunk.len() as u64, *size);
                    *size -= len;
                    let ch = chunk.split_to(len as usize);
                    if !chunk.is_empty() {
                        payload.unprocessed(chunk);
                    }
                    Ok(Async::Ready(Some(ch)))
                }
                None => {
                    if payload.eof && (*size != 0) {
                        Err(MultipartError::Incomplete)
                    } else {
                        Ok(Async::NotReady)
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
    ) -> Poll<Option<Bytes>, MultipartError> {
        let mut pos = 0;

        let len = payload.buf.len();
        if len == 0 {
            return if payload.eof {
                Err(MultipartError::Incomplete)
            } else {
                Ok(Async::NotReady)
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
                    return Ok(Async::NotReady);
                } else {
                    if &payload.buf[b_len..b_size] == boundary.as_bytes() {
                        // found boundary
                        return Ok(Async::Ready(None));
                    }
                }
            }
        }

        loop {
            return if let Some(idx) = twoway::find_bytes(&payload.buf[pos..], b"\r") {
                let cur = pos + idx;

                // check if we have enough data for boundary detection
                if cur + 4 > len {
                    if cur > 0 {
                        Ok(Async::Ready(Some(payload.buf.split_to(cur).freeze())))
                    } else {
                        Ok(Async::NotReady)
                    }
                } else {
                    // check boundary
                    if (&payload.buf[cur..cur + 2] == b"\r\n"
                        && &payload.buf[cur + 2..cur + 4] == b"--")
                        || (&payload.buf[cur..cur + 1] == b"\r"
                            && &payload.buf[cur + 1..cur + 3] == b"--")
                    {
                        if cur != 0 {
                            // return buffer
                            Ok(Async::Ready(Some(payload.buf.split_to(cur).freeze())))
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
                Ok(Async::Ready(Some(payload.buf.take().freeze())))
            };
        }
    }

    fn poll(&mut self, s: &Safety) -> Poll<Option<Bytes>, MultipartError> {
        if self.payload.is_none() {
            return Ok(Async::Ready(None));
        }

        let result = if let Some(payload) = self.payload.as_ref().unwrap().get_mut(s) {
            if !self.eof {
                let res = if let Some(ref mut len) = self.length {
                    InnerField::read_len(payload, len)?
                } else {
                    InnerField::read_stream(payload, &self.boundary)?
                };

                match res {
                    Async::NotReady => return Ok(Async::NotReady),
                    Async::Ready(Some(bytes)) => return Ok(Async::Ready(Some(bytes))),
                    Async::Ready(None) => self.eof = true,
                }
            }

            match payload.readline()? {
                None => Async::Ready(None),
                Some(line) => {
                    if line.as_ref() != b"\r\n" {
                        log::warn!("multipart field did not read all the data or it is malformed");
                    }
                    Async::Ready(None)
                }
            }
        } else {
            Async::NotReady
        };

        if Async::Ready(None) == result {
            self.payload.take();
        }
        Ok(result)
    }
}

struct PayloadRef {
    payload: Rc<UnsafeCell<PayloadBuffer>>,
}

impl PayloadRef {
    fn new(payload: PayloadBuffer) -> PayloadRef {
        PayloadRef {
            payload: Rc::new(payload.into()),
        }
    }

    fn get_mut<'a, 'b>(&'a self, s: &'b Safety) -> Option<&'a mut PayloadBuffer>
    where
        'a: 'b,
    {
        // Unsafe: Invariant is inforced by Safety Safety is used as ref counter,
        // only top most ref can have mutable access to payload.
        if s.current() {
            let payload: &mut PayloadBuffer = unsafe { &mut *self.payload.get() };
            Some(payload)
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

/// Counter. It tracks of number of clones of payloads and give access to
/// payload only to top most task panics if Safety get destroyed and it not top
/// most task.
#[derive(Debug)]
struct Safety {
    task: Option<Task>,
    level: usize,
    payload: Rc<PhantomData<bool>>,
    clean: Rc<Cell<bool>>,
}

impl Safety {
    fn new() -> Safety {
        let payload = Rc::new(PhantomData);
        Safety {
            task: None,
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
}

impl Clone for Safety {
    fn clone(&self) -> Safety {
        let payload = Rc::clone(&self.payload);
        Safety {
            task: Some(current_task()),
            level: Rc::strong_count(&payload),
            clean: self.clean.clone(),
            payload,
        }
    }
}

impl Drop for Safety {
    fn drop(&mut self) {
        // parent task is dead
        if Rc::strong_count(&self.payload) != self.level {
            self.clean.set(true);
        }
        if let Some(task) = self.task.take() {
            task.notify()
        }
    }
}

/// Payload buffer
struct PayloadBuffer {
    eof: bool,
    buf: BytesMut,
    stream: Box<dyn Stream<Item = Bytes, Error = PayloadError>>,
}

impl PayloadBuffer {
    /// Create new `PayloadBuffer` instance
    fn new<S>(stream: S) -> Self
    where
        S: Stream<Item = Bytes, Error = PayloadError> + 'static,
    {
        PayloadBuffer {
            eof: false,
            buf: BytesMut::new(),
            stream: Box::new(stream),
        }
    }

    fn poll_stream(&mut self) -> Result<(), PayloadError> {
        loop {
            match self.stream.poll()? {
                Async::Ready(Some(data)) => self.buf.extend_from_slice(&data),
                Async::Ready(None) => {
                    self.eof = true;
                    return Ok(());
                }
                Async::NotReady => return Ok(()),
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
    pub fn read_until(&mut self, line: &[u8]) -> Result<Option<Bytes>, MultipartError> {
        let res = twoway::find_bytes(&self.buf, line)
            .map(|idx| self.buf.split_to(idx + line.len()).freeze());

        if res.is_none() && self.eof {
            Err(MultipartError::Incomplete)
        } else {
            Ok(res)
        }
    }

    /// Read bytes until new line delimiter
    pub fn readline(&mut self) -> Result<Option<Bytes>, MultipartError> {
        self.read_until(b"\n")
    }

    /// Put unprocessed data back to the buffer
    pub fn unprocessed(&mut self, data: Bytes) {
        let buf = BytesMut::from(data);
        let buf = std::mem::replace(&mut self.buf, buf);
        self.buf.extend_from_slice(&buf);
    }
}

#[cfg(test)]
mod tests {
    use actix_http::h1::Payload;
    use bytes::Bytes;
    use futures::unsync::mpsc;

    use super::*;
    use actix_web::http::header::{DispositionParam, DispositionType};
    use actix_web::test::run_on;

    #[test]
    fn test_boundary() {
        let headers = HeaderMap::new();
        match Multipart::boundary(&headers) {
            Err(MultipartError::NoContentType) => (),
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("test"),
        );

        match Multipart::boundary(&headers) {
            Err(MultipartError::ParseContentType) => (),
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("multipart/mixed"),
        );
        match Multipart::boundary(&headers) {
            Err(MultipartError::Boundary) => (),
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
        impl Stream<Item = Bytes, Error = PayloadError>,
    ) {
        let (tx, rx) = mpsc::unbounded();

        (tx, rx.map_err(|_| panic!()).and_then(|res| res))
    }

    #[test]
    fn test_multipart() {
        run_on(|| {
            let (sender, payload) = create_stream();

            let bytes = Bytes::from(
                "testasdadsad\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
                 Content-Disposition: form-data; name=\"file\"; filename=\"fn.txt\"\r\n\
                 Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
                 test\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
                 Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
                 data\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0--\r\n",
            );
            sender.unbounded_send(Ok(bytes)).unwrap();

            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static(
                    "multipart/mixed; boundary=\"abbc761f78ff4d7cb7573b5a23f96ef0\"",
                ),
            );

            let mut multipart = Multipart::new(&headers, payload);
            match multipart.poll().unwrap() {
                Async::Ready(Some(mut field)) => {
                    let cd = field.content_disposition().unwrap();
                    assert_eq!(cd.disposition, DispositionType::FormData);
                    assert_eq!(cd.parameters[0], DispositionParam::Name("file".into()));

                    assert_eq!(field.content_type().type_(), mime::TEXT);
                    assert_eq!(field.content_type().subtype(), mime::PLAIN);

                    match field.poll().unwrap() {
                        Async::Ready(Some(chunk)) => assert_eq!(chunk, "test"),
                        _ => unreachable!(),
                    }
                    match field.poll().unwrap() {
                        Async::Ready(None) => (),
                        _ => unreachable!(),
                    }
                }
                _ => unreachable!(),
            }

            match multipart.poll().unwrap() {
                Async::Ready(Some(mut field)) => {
                    assert_eq!(field.content_type().type_(), mime::TEXT);
                    assert_eq!(field.content_type().subtype(), mime::PLAIN);

                    match field.poll() {
                        Ok(Async::Ready(Some(chunk))) => assert_eq!(chunk, "data"),
                        _ => unreachable!(),
                    }
                    match field.poll() {
                        Ok(Async::Ready(None)) => (),
                        _ => unreachable!(),
                    }
                }
                _ => unreachable!(),
            }

            match multipart.poll().unwrap() {
                Async::Ready(None) => (),
                _ => unreachable!(),
            }
        });
    }

    #[test]
    fn test_stream() {
        run_on(|| {
            let (sender, payload) = create_stream();

            let bytes = Bytes::from(
                "testasdadsad\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
                 Content-Disposition: form-data; name=\"file\"; filename=\"fn.txt\"\r\n\
                 Content-Type: text/plain; charset=utf-8\r\n\r\n\
                 test\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
                 Content-Type: text/plain; charset=utf-8\r\n\r\n\
                 data\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0--\r\n",
            );
            sender.unbounded_send(Ok(bytes)).unwrap();

            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static(
                    "multipart/mixed; boundary=\"abbc761f78ff4d7cb7573b5a23f96ef0\"",
                ),
            );

            let mut multipart = Multipart::new(&headers, payload);
            match multipart.poll().unwrap() {
                Async::Ready(Some(mut field)) => {
                    let cd = field.content_disposition().unwrap();
                    assert_eq!(cd.disposition, DispositionType::FormData);
                    assert_eq!(cd.parameters[0], DispositionParam::Name("file".into()));

                    assert_eq!(field.content_type().type_(), mime::TEXT);
                    assert_eq!(field.content_type().subtype(), mime::PLAIN);

                    match field.poll().unwrap() {
                        Async::Ready(Some(chunk)) => assert_eq!(chunk, "test"),
                        _ => unreachable!(),
                    }
                    match field.poll().unwrap() {
                        Async::Ready(None) => (),
                        _ => unreachable!(),
                    }
                }
                _ => unreachable!(),
            }

            match multipart.poll().unwrap() {
                Async::Ready(Some(mut field)) => {
                    assert_eq!(field.content_type().type_(), mime::TEXT);
                    assert_eq!(field.content_type().subtype(), mime::PLAIN);

                    match field.poll() {
                        Ok(Async::Ready(Some(chunk))) => assert_eq!(chunk, "data"),
                        _ => unreachable!(),
                    }
                    match field.poll() {
                        Ok(Async::Ready(None)) => (),
                        _ => unreachable!(),
                    }
                }
                _ => unreachable!(),
            }

            match multipart.poll().unwrap() {
                Async::Ready(None) => (),
                _ => unreachable!(),
            }
        });
    }

    #[test]
    fn test_basic() {
        run_on(|| {
            let (_, payload) = Payload::create(false);
            let mut payload = PayloadBuffer::new(payload);

            assert_eq!(payload.buf.len(), 0);
            payload.poll_stream().unwrap();
            assert_eq!(None, payload.read_max(1).unwrap());
        })
    }

    #[test]
    fn test_eof() {
        run_on(|| {
            let (mut sender, payload) = Payload::create(false);
            let mut payload = PayloadBuffer::new(payload);

            assert_eq!(None, payload.read_max(4).unwrap());
            sender.feed_data(Bytes::from("data"));
            sender.feed_eof();
            payload.poll_stream().unwrap();

            assert_eq!(Some(Bytes::from("data")), payload.read_max(4).unwrap());
            assert_eq!(payload.buf.len(), 0);
            assert!(payload.read_max(1).is_err());
            assert!(payload.eof);
        })
    }

    #[test]
    fn test_err() {
        run_on(|| {
            let (mut sender, payload) = Payload::create(false);
            let mut payload = PayloadBuffer::new(payload);
            assert_eq!(None, payload.read_max(1).unwrap());
            sender.set_error(PayloadError::Incomplete(None));
            payload.poll_stream().err().unwrap();
        })
    }

    #[test]
    fn test_readmax() {
        run_on(|| {
            let (mut sender, payload) = Payload::create(false);
            let mut payload = PayloadBuffer::new(payload);

            sender.feed_data(Bytes::from("line1"));
            sender.feed_data(Bytes::from("line2"));
            payload.poll_stream().unwrap();
            assert_eq!(payload.buf.len(), 10);

            assert_eq!(Some(Bytes::from("line1")), payload.read_max(5).unwrap());
            assert_eq!(payload.buf.len(), 5);

            assert_eq!(Some(Bytes::from("line2")), payload.read_max(5).unwrap());
            assert_eq!(payload.buf.len(), 0);
        })
    }

    #[test]
    fn test_readexactly() {
        run_on(|| {
            let (mut sender, payload) = Payload::create(false);
            let mut payload = PayloadBuffer::new(payload);

            assert_eq!(None, payload.read_exact(2));

            sender.feed_data(Bytes::from("line1"));
            sender.feed_data(Bytes::from("line2"));
            payload.poll_stream().unwrap();

            assert_eq!(Some(Bytes::from_static(b"li")), payload.read_exact(2));
            assert_eq!(payload.buf.len(), 8);

            assert_eq!(Some(Bytes::from_static(b"ne1l")), payload.read_exact(4));
            assert_eq!(payload.buf.len(), 4);
        })
    }

    #[test]
    fn test_readuntil() {
        run_on(|| {
            let (mut sender, payload) = Payload::create(false);
            let mut payload = PayloadBuffer::new(payload);

            assert_eq!(None, payload.read_until(b"ne").unwrap());

            sender.feed_data(Bytes::from("line1"));
            sender.feed_data(Bytes::from("line2"));
            payload.poll_stream().unwrap();

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
        })
    }
}
