//! Multipart requests support
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::{cmp, fmt};

use bytes::Bytes;
use futures::task::{current as current_task, Task};
use futures::{Async, Poll, Stream};
use http::header::{self, ContentDisposition, HeaderMap, HeaderName, HeaderValue};
use http::HttpTryFrom;
use httparse;
use mime;

use error::{MultipartError, ParseError, PayloadError};
use payload::PayloadHelper;

const MAX_HEADERS: usize = 32;

/// The server-side implementation of `multipart/form-data` requests.
///
/// This will parse the incoming stream into `MultipartItem` instances via its
/// Stream implementation.
/// `MultipartItem::Field` contains multipart field. `MultipartItem::Multipart`
/// is used for nested multipart streams.
pub struct Multipart<S> {
    safety: Safety,
    error: Option<MultipartError>,
    inner: Option<Rc<RefCell<InnerMultipart<S>>>>,
}

///
pub enum MultipartItem<S> {
    /// Multipart field
    Field(Field<S>),
    /// Nested multipart stream
    Nested(Multipart<S>),
}

enum InnerMultipartItem<S> {
    None,
    Field(Rc<RefCell<InnerField<S>>>),
    Multipart(Rc<RefCell<InnerMultipart<S>>>),
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

struct InnerMultipart<S> {
    payload: PayloadRef<S>,
    boundary: String,
    state: InnerState,
    item: InnerMultipartItem<S>,
}

impl Multipart<()> {
    /// Extract boundary info from headers.
    pub fn boundary(headers: &HeaderMap) -> Result<String, MultipartError> {
        if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
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

impl<S> Multipart<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    /// Create multipart instance for boundary.
    pub fn new(boundary: Result<String, MultipartError>, stream: S) -> Multipart<S> {
        match boundary {
            Ok(boundary) => Multipart {
                error: None,
                safety: Safety::new(),
                inner: Some(Rc::new(RefCell::new(InnerMultipart {
                    boundary,
                    payload: PayloadRef::new(PayloadHelper::new(stream)),
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
}

impl<S> Stream for Multipart<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    type Item = MultipartItem<S>;
    type Error = MultipartError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some(err) = self.error.take() {
            Err(err)
        } else if self.safety.current() {
            self.inner.as_mut().unwrap().borrow_mut().poll(&self.safety)
        } else {
            Ok(Async::NotReady)
        }
    }
}

impl<S> InnerMultipart<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    fn read_headers(payload: &mut PayloadHelper<S>) -> Poll<HeaderMap, MultipartError> {
        match payload.read_until(b"\r\n\r\n")? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(None) => Err(MultipartError::Incomplete),
            Async::Ready(Some(bytes)) => {
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
                        Ok(Async::Ready(headers))
                    }
                    Ok(httparse::Status::Partial) => Err(ParseError::Header.into()),
                    Err(err) => Err(ParseError::from(err).into()),
                }
            }
        }
    }

    fn read_boundary(
        payload: &mut PayloadHelper<S>, boundary: &str,
    ) -> Poll<bool, MultipartError> {
        // TODO: need to read epilogue
        match payload.readline()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(None) => Err(MultipartError::Incomplete),
            Async::Ready(Some(chunk)) => {
                if chunk.len() == boundary.len() + 4
                    && &chunk[..2] == b"--"
                    && &chunk[2..boundary.len() + 2] == boundary.as_bytes()
                {
                    Ok(Async::Ready(false))
                } else if chunk.len() == boundary.len() + 6
                    && &chunk[..2] == b"--"
                    && &chunk[2..boundary.len() + 2] == boundary.as_bytes()
                    && &chunk[boundary.len() + 2..boundary.len() + 4] == b"--"
                {
                    Ok(Async::Ready(true))
                } else {
                    Err(MultipartError::Boundary)
                }
            }
        }
    }

    fn skip_until_boundary(
        payload: &mut PayloadHelper<S>, boundary: &str,
    ) -> Poll<bool, MultipartError> {
        let mut eof = false;
        loop {
            match payload.readline()? {
                Async::Ready(Some(chunk)) => {
                    if chunk.is_empty() {
                        //ValueError("Could not find starting boundary %r"
                        //% (self._boundary))
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
                Async::NotReady => return Ok(Async::NotReady),
                Async::Ready(None) => return Err(MultipartError::Incomplete),
            }
        }
        Ok(Async::Ready(eof))
    }

    fn poll(
        &mut self, safety: &Safety,
    ) -> Poll<Option<MultipartItem<S>>, MultipartError> {
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
                        InnerMultipartItem::Multipart(ref mut multipart) => {
                            match multipart.borrow_mut().poll(safety)? {
                                Async::NotReady => return Ok(Async::NotReady),
                                Async::Ready(Some(_)) => continue,
                                Async::Ready(None) => true,
                            }
                        }
                        _ => false,
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
                            Async::Ready(eof) => {
                                if eof {
                                    self.state = InnerState::Eof;
                                    return Ok(Async::Ready(None));
                                } else {
                                    self.state = InnerState::Headers;
                                }
                            }
                            Async::NotReady => return Ok(Async::NotReady),
                        }
                    }
                    // read boundary
                    InnerState::Boundary => {
                        match InnerMultipart::read_boundary(payload, &self.boundary)? {
                            Async::NotReady => return Ok(Async::NotReady),
                            Async::Ready(eof) => {
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
                    if let Async::Ready(headers) = InnerMultipart::read_headers(payload)?
                    {
                        self.state = InnerState::Boundary;
                        headers
                    } else {
                        return Ok(Async::NotReady);
                    }
                } else {
                    unreachable!()
                }
            } else {
                debug!("NotReady: field is in flight");
                return Ok(Async::NotReady);
            };

            // content disposition
            // RFC 7578: 'Each part MUST contain a Content-Disposition header field
            // where the disposition type is "form-data".'
            let cd = ContentDisposition::from_raw(
                headers.get(::http::header::CONTENT_DISPOSITION)
            ).map_err(|_| MultipartError::ParseContentDisposition)?;

            // content type
            let mut mt = mime::APPLICATION_OCTET_STREAM;
            if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
                if let Ok(content_type) = content_type.to_str() {
                    if let Ok(ct) = content_type.parse::<mime::Mime>() {
                        mt = ct;
                    }
                }
            }

            self.state = InnerState::Boundary;

            // nested multipart stream
            if mt.type_() == mime::MULTIPART {
                let inner = if let Some(boundary) = mt.get_param(mime::BOUNDARY) {
                    Rc::new(RefCell::new(InnerMultipart {
                        payload: self.payload.clone(),
                        boundary: boundary.as_str().to_owned(),
                        state: InnerState::FirstBoundary,
                        item: InnerMultipartItem::None,
                    }))
                } else {
                    return Err(MultipartError::Boundary);
                };

                self.item = InnerMultipartItem::Multipart(Rc::clone(&inner));

                Ok(Async::Ready(Some(MultipartItem::Nested(Multipart {
                    safety: safety.clone(),
                    error: None,
                    inner: Some(inner),
                }))))
            } else {
                let field = Rc::new(RefCell::new(InnerField::new(
                    self.payload.clone(),
                    self.boundary.clone(),
                    &headers,
                )?));
                self.item = InnerMultipartItem::Field(Rc::clone(&field));

                Ok(Async::Ready(Some(MultipartItem::Field(Field::new(
                    safety.clone(),
                    headers,
                    cd,
                    mt,
                    field,
                )))))
            }
        }
    }
}

impl<S> Drop for InnerMultipart<S> {
    fn drop(&mut self) {
        // InnerMultipartItem::Field has to be dropped first because of Safety.
        self.item = InnerMultipartItem::None;
    }
}

/// A single field in a multipart stream
pub struct Field<S> {
    cd: ContentDisposition,
    ct: mime::Mime,
    headers: HeaderMap,
    inner: Rc<RefCell<InnerField<S>>>,
    safety: Safety,
}

impl<S> Field<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    fn new(
        safety: Safety, headers: HeaderMap, cd: ContentDisposition, ct: mime::Mime,
        inner: Rc<RefCell<InnerField<S>>>,
    ) -> Self {
        Field {
            cd,
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

    /// Get the content disposition of the field
    pub fn content_disposition(&self) -> &ContentDisposition {
        &self.cd
    }
}

impl<S> Stream for Field<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    type Item = Bytes;
    type Error = MultipartError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if self.safety.current() {
            self.inner.borrow_mut().poll(&self.safety)
        } else {
            Ok(Async::NotReady)
        }
    }
}

impl<S> fmt::Debug for Field<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = writeln!(f, "\nMultipartField: {}", self.ct);
        let _ = writeln!(f, "  boundary: {}", self.inner.borrow().boundary);
        let _ = writeln!(f, "  headers:");
        for (key, val) in self.headers.iter() {
            let _ = writeln!(f, "    {:?}: {:?}", key, val);
        }
        res
    }
}

struct InnerField<S> {
    payload: Option<PayloadRef<S>>,
    boundary: String,
    eof: bool,
    length: Option<u64>,
}

impl<S> InnerField<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    fn new(
        payload: PayloadRef<S>, boundary: String, headers: &HeaderMap,
    ) -> Result<InnerField<S>, PayloadError> {
        let len = if let Some(len) = headers.get(header::CONTENT_LENGTH) {
            if let Ok(s) = len.to_str() {
                if let Ok(len) = s.parse::<u64>() {
                    Some(len)
                } else {
                    return Err(PayloadError::Incomplete);
                }
            } else {
                return Err(PayloadError::Incomplete);
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
        payload: &mut PayloadHelper<S>, size: &mut u64,
    ) -> Poll<Option<Bytes>, MultipartError> {
        if *size == 0 {
            Ok(Async::Ready(None))
        } else {
            match payload.readany() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Ok(Async::Ready(None)) => Err(MultipartError::Incomplete),
                Ok(Async::Ready(Some(mut chunk))) => {
                    let len = cmp::min(chunk.len() as u64, *size);
                    *size -= len;
                    let ch = chunk.split_to(len as usize);
                    if !chunk.is_empty() {
                        payload.unread_data(chunk);
                    }
                    Ok(Async::Ready(Some(ch)))
                }
                Err(err) => Err(err.into()),
            }
        }
    }

    /// Reads content chunk of body part with unknown length.
    /// The `Content-Length` header for body part is not necessary.
    fn read_stream(
        payload: &mut PayloadHelper<S>, boundary: &str,
    ) -> Poll<Option<Bytes>, MultipartError> {
        match payload.read_until(b"\r")? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(None) => Err(MultipartError::Incomplete),
            Async::Ready(Some(mut chunk)) => {
                if chunk.len() == 1 {
                    payload.unread_data(chunk);
                    match payload.read_exact(boundary.len() + 4)? {
                        Async::NotReady => Ok(Async::NotReady),
                        Async::Ready(None) => Err(MultipartError::Incomplete),
                        Async::Ready(Some(mut chunk)) => {
                            if &chunk[..2] == b"\r\n"
                                && &chunk[2..4] == b"--"
                                && &chunk[4..] == boundary.as_bytes()
                            {
                                payload.unread_data(chunk);
                                Ok(Async::Ready(None))
                            } else {
                                // \r might be part of data stream
                                let ch = chunk.split_to(1);
                                payload.unread_data(chunk);
                                Ok(Async::Ready(Some(ch)))
                            }
                        }
                    }
                } else {
                    let to = chunk.len() - 1;
                    let ch = chunk.split_to(to);
                    payload.unread_data(chunk);
                    Ok(Async::Ready(Some(ch)))
                }
            }
        }
    }

    fn poll(&mut self, s: &Safety) -> Poll<Option<Bytes>, MultipartError> {
        if self.payload.is_none() {
            return Ok(Async::Ready(None));
        }

        let result = if let Some(payload) = self.payload.as_ref().unwrap().get_mut(s) {
            let res = if let Some(ref mut len) = self.length {
                InnerField::read_len(payload, len)?
            } else {
                InnerField::read_stream(payload, &self.boundary)?
            };

            match res {
                Async::NotReady => Async::NotReady,
                Async::Ready(Some(bytes)) => Async::Ready(Some(bytes)),
                Async::Ready(None) => {
                    self.eof = true;
                    match payload.readline()? {
                        Async::NotReady => Async::NotReady,
                        Async::Ready(None) => Async::Ready(None),
                        Async::Ready(Some(line)) => {
                            if line.as_ref() != b"\r\n" {
                                warn!("multipart field did not read all the data or it is malformed");
                            }
                            Async::Ready(None)
                        }
                    }
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

struct PayloadRef<S> {
    payload: Rc<PayloadHelper<S>>,
}

impl<S> PayloadRef<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    fn new(payload: PayloadHelper<S>) -> PayloadRef<S> {
        PayloadRef {
            payload: Rc::new(payload),
        }
    }

    fn get_mut<'a, 'b>(&'a self, s: &'b Safety) -> Option<&'a mut PayloadHelper<S>>
    where
        'a: 'b,
    {
        if s.current() {
            let payload: &mut PayloadHelper<S> =
                unsafe { &mut *(self.payload.as_ref() as *const _ as *mut _) };
            Some(payload)
        } else {
            None
        }
    }
}

impl<S> Clone for PayloadRef<S> {
    fn clone(&self) -> PayloadRef<S> {
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
}

impl Safety {
    fn new() -> Safety {
        let payload = Rc::new(PhantomData);
        Safety {
            task: None,
            level: Rc::strong_count(&payload),
            payload,
        }
    }

    fn current(&self) -> bool {
        Rc::strong_count(&self.payload) == self.level
    }
}

impl Clone for Safety {
    fn clone(&self) -> Safety {
        let payload = Rc::clone(&self.payload);
        Safety {
            task: Some(current_task()),
            level: Rc::strong_count(&payload),
            payload,
        }
    }
}

impl Drop for Safety {
    fn drop(&mut self) {
        // parent task is dead
        if Rc::strong_count(&self.payload) != self.level {
            panic!("Safety get dropped but it is not from top-most task");
        }
        if let Some(task) = self.task.take() {
            task.notify()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::future::{lazy, result};
    use payload::{Payload, PayloadWriter};
    use tokio::runtime::current_thread::Runtime;

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

    #[test]
    fn test_multipart() {
        Runtime::new()
            .unwrap()
            .block_on(lazy(|| {
                let (mut sender, payload) = Payload::new(false);

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
                 --abbc761f78ff4d7cb7573b5a23f96ef0--\r\n");
                sender.feed_data(bytes);

                let mut multipart = Multipart::new(
                    Ok("abbc761f78ff4d7cb7573b5a23f96ef0".to_owned()),
                    payload,
                );
                match multipart.poll() {
                    Ok(Async::Ready(Some(item))) => match item {
                        MultipartItem::Field(mut field) => {
                            {
                                use http::header::{DispositionType, DispositionParam};
                                let cd = field.content_disposition();
                                assert_eq!(cd.disposition, DispositionType::Ext("form-data".into()));
                                assert_eq!(cd.parameters[0], DispositionParam::Ext("name".into(), "file".into()));
                            }
                            assert_eq!(field.content_type().type_(), mime::TEXT);
                            assert_eq!(field.content_type().subtype(), mime::PLAIN);

                            match field.poll() {
                                Ok(Async::Ready(Some(chunk))) => {
                                    assert_eq!(chunk, "test")
                                }
                                _ => unreachable!(),
                            }
                            match field.poll() {
                                Ok(Async::Ready(None)) => (),
                                _ => unreachable!(),
                            }
                        }
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                }

                match multipart.poll() {
                    Ok(Async::Ready(Some(item))) => match item {
                        MultipartItem::Field(mut field) => {
                            assert_eq!(field.content_type().type_(), mime::TEXT);
                            assert_eq!(field.content_type().subtype(), mime::PLAIN);

                            match field.poll() {
                                Ok(Async::Ready(Some(chunk))) => {
                                    assert_eq!(chunk, "data")
                                }
                                _ => unreachable!(),
                            }
                            match field.poll() {
                                Ok(Async::Ready(None)) => (),
                                _ => unreachable!(),
                            }
                        }
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                }

                match multipart.poll() {
                    Ok(Async::Ready(None)) => (),
                    _ => unreachable!(),
                }

                let res: Result<(), ()> = Ok(());
                result(res)
            }))
            .unwrap();
    }
}
