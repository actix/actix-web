//! Multipart requests support
use std::{cmp, fmt};
use std::rc::Rc;
use std::cell::RefCell;
use std::marker::PhantomData;

use mime;
use httparse;
use bytes::Bytes;
use http::HttpTryFrom;
use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use futures::{Async, Future, Stream, Poll};
use futures::task::{Task, current as current_task};

use error::{ParseError, PayloadError, MultipartError};
use payload::Payload;
use httprequest::HttpRequest;

const MAX_HEADERS: usize = 32;

/// The server-side implementation of `multipart/form-data` requests.
///
/// This will parse the incoming stream into `MultipartItem` instances via its
/// Stream implementation.
/// `MultipartItem::Field` contains multipart field. `MultipartItem::Multipart`
/// is used for nested multipart streams.
#[derive(Debug)]
pub struct Multipart {
    safety: Safety,
    error: Option<MultipartError>,
    inner: Option<Rc<RefCell<InnerMultipart>>>,
}

///
#[derive(Debug)]
pub enum MultipartItem {
    /// Multipart field
    Field(Field),
    /// Nested multipart stream
    Nested(Multipart),
}

#[derive(Debug)]
enum InnerMultipartItem {
    None,
    Field(Rc<RefCell<InnerField>>),
    Multipart(Rc<RefCell<InnerMultipart>>),
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

#[derive(Debug)]
struct InnerMultipart {
    payload: PayloadRef,
    boundary: String,
    state: InnerState,
    item: InnerMultipartItem,
}

impl Multipart {

    /// Create multipart instance for boundary.
    pub fn new(boundary: String, payload: Payload) -> Multipart {
        Multipart {
            error: None,
            safety: Safety::new(),
            inner: Some(Rc::new(RefCell::new(
                InnerMultipart {
                    payload: PayloadRef::new(payload),
                    boundary: boundary,
                    state: InnerState::FirstBoundary,
                    item: InnerMultipartItem::None,
                })))
        }
    }

    /// Create multipart instance for request.
    pub fn from_request<S>(req: &mut HttpRequest<S>) -> Multipart {
        match Multipart::boundary(req.headers()) {
            Ok(boundary) => Multipart::new(boundary, req.payload().clone()),
            Err(err) =>
                Multipart {
                    error: Some(err),
                    safety: Safety::new(),
                    inner: None,
                }
        }
    }

    // /// Create multipart instance for client response.
    // pub fn from_response(resp: &mut ClientResponse) -> Multipart {
    //   match Multipart::boundary(resp.headers()) {
    //         Ok(boundary) => Multipart::new(boundary, resp.payload().clone()),
    //         Err(err) =>
    //             Multipart {
    //                 error: Some(err),
    //                 safety: Safety::new(),
    //                 inner: None,
    //             }
    //     }
    // }

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

impl Stream for Multipart {
    type Item = MultipartItem;
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

impl InnerMultipart {

    fn read_headers(payload: &mut Payload) -> Poll<HeaderMap, MultipartError>
    {
        match payload.readuntil(b"\r\n\r\n").poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(bytes) => {
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
                                    return Err(ParseError::Header.into())
                                }
                            } else {
                                return Err(ParseError::Header.into())
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

    fn read_boundary(payload: &mut Payload, boundary: &str) -> Poll<bool, MultipartError>
    {
        // TODO: need to read epilogue
        match payload.readline().poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(chunk) => {
                if chunk.len() == boundary.len() + 4 &&
                    &chunk[..2] == b"--" &&
                    &chunk[2..boundary.len()+2] == boundary.as_bytes()
                {
                    Ok(Async::Ready(false))
                } else if chunk.len() == boundary.len() + 6 &&
                    &chunk[..2] == b"--" &&
                    &chunk[2..boundary.len()+2] == boundary.as_bytes() &&
                    &chunk[boundary.len()+2..boundary.len()+4] == b"--"
                {
                    Ok(Async::Ready(true))
                } else {
                    Err(MultipartError::Boundary)
                }
            }
        }
    }

    fn skip_until_boundary(payload: &mut Payload, boundary: &str) -> Poll<bool, MultipartError>
    {
        let mut eof = false;
        loop {
            if let Async::Ready(chunk) = payload.readline().poll()? {
                if chunk.is_empty() {
                    //ValueError("Could not find starting boundary %r"
                    //% (self._boundary))
                }
                if chunk.len() < boundary.len() {
                    continue
                }
                if &chunk[..2] == b"--" && &chunk[2..chunk.len()-2] == boundary.as_bytes() {
                    break;
                } else {
                    if chunk.len() < boundary.len() + 2{
                        continue
                    }
                    let b: &[u8] = boundary.as_ref();
                    if &chunk[..boundary.len()] == b &&
                        &chunk[boundary.len()..boundary.len()+2] == b"--" {
                            eof = true;
                            break;
                        }
                }
            } else {
                return Ok(Async::NotReady)
            }
        }
        Ok(Async::Ready(eof))
    }

    fn poll(&mut self, safety: &Safety) -> Poll<Option<MultipartItem>, MultipartError> {
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
                                Async::NotReady => {
                                    return Ok(Async::NotReady)
                                }
                                Async::Ready(Some(_)) =>
                                    continue,
                                Async::Ready(None) =>
                                    true,
                            }
                        }
                        InnerMultipartItem::Multipart(ref mut multipart) => {
                            match multipart.borrow_mut().poll(safety)? {
                                Async::NotReady =>
                                    return Ok(Async::NotReady),
                                Async::Ready(Some(_)) =>
                                    continue,
                                Async::Ready(None) =>
                                    true,
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
                        if let Async::Ready(eof) =
                            InnerMultipart::skip_until_boundary(payload, &self.boundary)?
                        {
                            if eof {
                                self.state = InnerState::Eof;
                                return Ok(Async::Ready(None));
                            } else {
                                self.state = InnerState::Headers;
                            }
                        } else {
                            return Ok(Async::NotReady)
                        }
                    }
                    // read boundary
                    InnerState::Boundary => {
                        match InnerMultipart::read_boundary(payload, &self.boundary)? {
                            Async::NotReady => {
                                return Ok(Async::NotReady)
                            }
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
                    if let Async::Ready(headers) = InnerMultipart::read_headers(payload)? {
                        self.state = InnerState::Boundary;
                        headers
                    } else {
                        return Ok(Async::NotReady)
                    }
                } else {
                    unreachable!()
                }
            } else {
                debug!("NotReady: field is in flight");
                return Ok(Async::NotReady)
            };

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
                    Rc::new(RefCell::new(
                        InnerMultipart {
                            payload: self.payload.clone(),
                            boundary: boundary.as_str().to_owned(),
                            state: InnerState::FirstBoundary,
                            item: InnerMultipartItem::None,
                        }))
                } else {
                    return Err(MultipartError::Boundary)
                };

                self.item = InnerMultipartItem::Multipart(Rc::clone(&inner));

                Ok(Async::Ready(Some(
                    MultipartItem::Nested(
                        Multipart{safety: safety.clone(),
                                  error: None,
                                  inner: Some(inner)}))))
            } else {
                let field = Rc::new(RefCell::new(InnerField::new(
                    self.payload.clone(), self.boundary.clone(), &headers)?));
                self.item = InnerMultipartItem::Field(Rc::clone(&field));

                Ok(Async::Ready(Some(
                    MultipartItem::Field(
                        Field::new(safety.clone(), headers, mt, field)))))
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

    fn new(safety: Safety, headers: HeaderMap,
           ct: mime::Mime, inner: Rc<RefCell<InnerField>>) -> Self {
        Field {
            ct: ct,
            headers: headers,
            inner: inner,
            safety: safety,
        }
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn content_type(&self) -> &mime::Mime {
        &self.ct
    }
}

impl Stream for Field {
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

impl fmt::Debug for Field {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(f, "\nMultipartField: {}\n", self.ct);
        let _ = write!(f, "  boundary: {}\n", self.inner.borrow().boundary);
        let _ = write!(f, "  headers:\n");
        for key in self.headers.keys() {
            let vals: Vec<_> = self.headers.get_all(key).iter().collect();
            if vals.len() > 1 {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals);
            } else {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals[0]);
            }
        }
        res
    }
}

#[derive(Debug)]
struct InnerField {
    payload: Option<PayloadRef>,
    boundary: String,
    eof: bool,
    length: Option<u64>,
}

impl InnerField {

    fn new(payload: PayloadRef, boundary: String, headers: &HeaderMap)
           -> Result<InnerField, PayloadError>
    {
        let len = if let Some(len) = headers.get(header::CONTENT_LENGTH) {
            if let Ok(s) = len.to_str() {
                if let Ok(len) = s.parse::<u64>() {
                    Some(len)
                } else {
                    return Err(PayloadError::Incomplete)
                }
            } else {
                return Err(PayloadError::Incomplete)
            }
        } else {
            None
        };

        Ok(InnerField {
            payload: Some(payload),
            boundary: boundary,
            eof: false,
            length: len })
    }

    /// Reads body part content chunk of the specified size.
    /// The body part must has `Content-Length` header with proper value.
    fn read_len(payload: &mut Payload, size: &mut u64) -> Poll<Option<Bytes>, MultipartError>
    {
        if *size == 0 {
            Ok(Async::Ready(None))
        } else {
            match payload.poll() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Ok(Async::Ready(None)) => Ok(Async::Ready(None)),
                Ok(Async::Ready(Some(mut chunk))) => {
                    let len = cmp::min(chunk.len() as u64, *size);
                    *size -= len;
                    let ch = chunk.split_to(len as usize);
                    if !chunk.is_empty() {
                        payload.unread_data(chunk);
                    }
                    Ok(Async::Ready(Some(ch)))
                },
                Err(err) => Err(err.into())
            }
        }
    }

    /// Reads content chunk of body part with unknown length.
    /// The `Content-Length` header for body part is not necessary.
    fn read_stream(payload: &mut Payload, boundary: &str) -> Poll<Option<Bytes>, MultipartError>
    {
        match payload.readuntil(b"\r").poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(mut chunk) => {
                if chunk.len() == 1 {
                    payload.unread_data(chunk);
                    match payload.readexactly(boundary.len() + 4).poll()? {
                        Async::NotReady => Ok(Async::NotReady),
                        Async::Ready(chunk) => {
                            if &chunk[..2] == b"\r\n" && &chunk[2..4] == b"--" &&
                                &chunk[4..] == boundary.as_bytes()
                            {
                                payload.unread_data(chunk);
                                Ok(Async::Ready(None))
                            } else {
                                Ok(Async::Ready(Some(chunk)))
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
            return Ok(Async::Ready(None))
        }
        if self.eof {
            if let Some(payload) = self.payload.as_ref().unwrap().get_mut(s) {
                match payload.readline().poll()? {
                    Async::NotReady =>
                        return Ok(Async::NotReady),
                    Async::Ready(chunk) => {
                        assert_eq!(
                            chunk.as_ref(), b"\r\n",
                            "reader did not read all the data or it is malformed");
                    }
                }
            } else {
                return Ok(Async::NotReady);
            }

            self.payload.take();
            return Ok(Async::Ready(None))
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
                    match payload.readline().poll()? {
                        Async::NotReady => Async::NotReady,
                        Async::Ready(chunk) => {
                            assert_eq!(
                                chunk.as_ref(), b"\r\n",
                                "reader did not read all the data or it is malformed");
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

#[derive(Debug)]
struct PayloadRef {
    task: Option<Task>,
    payload: Rc<Payload>,
}

impl PayloadRef {
    fn new(payload: Payload) -> PayloadRef {
        PayloadRef {
            task: None,
            payload: Rc::new(payload),
        }
    }

    fn get_mut<'a, 'b>(&'a self, s: &'b Safety) -> Option<&'a mut Payload>
        where 'a: 'b
    {
        if s.current() {
            let payload: &mut Payload = unsafe {
                &mut *(self.payload.as_ref() as *const _ as *mut _)};
            Some(payload)
        } else {
            None
        }
    }
}

impl Clone for PayloadRef {
    fn clone(&self) -> PayloadRef {
        PayloadRef {
            task: Some(current_task()),
            payload: Rc::clone(&self.payload),
        }
    }
}

/// Counter. It tracks of number of clones of payloads and give access to payload only
/// to top most task panics if Safety get destroyed and it not top most task.
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
            payload: payload,
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
            payload: payload,
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
    use tokio_core::reactor::Core;
    use payload::{Payload, PayloadWriter};

    #[test]
    fn test_boundary() {
        let headers = HeaderMap::new();
        match Multipart::boundary(&headers) {
            Err(MultipartError::NoContentType) => (),
            _ => panic!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("test"));

        match Multipart::boundary(&headers) {
            Err(MultipartError::ParseContentType) => (),
            _ => panic!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("multipart/mixed"));
        match Multipart::boundary(&headers) {
            Err(MultipartError::Boundary) => (),
            _ => panic!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(
                "multipart/mixed; boundary=\"5c02368e880e436dab70ed54e1c58209\""));

        assert_eq!(Multipart::boundary(&headers).unwrap(),
                   "5c02368e880e436dab70ed54e1c58209");
    }

    #[test]
    fn test_multipart() {
        Core::new().unwrap().run(lazy(|| {
            let (mut sender, payload) = Payload::new(false);

            let bytes = Bytes::from(
                "testasdadsad\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
                 Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
                 test\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
                 Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
                 data\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0--\r\n");
            sender.feed_data(bytes);

            let mut multipart = Multipart::new(
                "abbc761f78ff4d7cb7573b5a23f96ef0".to_owned(), payload);
            match multipart.poll() {
                Ok(Async::Ready(Some(item))) => {
                    match item {
                        MultipartItem::Field(mut field) => {
                            assert_eq!(field.content_type().type_(), mime::TEXT);
                            assert_eq!(field.content_type().subtype(), mime::PLAIN);

                            match field.poll() {
                                Ok(Async::Ready(Some(chunk))) =>
                                    assert_eq!(chunk, "test"),
                                _ => unreachable!()
                            }
                            match field.poll() {
                                Ok(Async::Ready(None)) => (),
                                _ => unreachable!()
                            }
                        },
                        _ => unreachable!()
                    }
                }
                _ => unreachable!()
            }

            match multipart.poll() {
                Ok(Async::Ready(Some(item))) => {
                    match item {
                        MultipartItem::Field(mut field) => {
                            assert_eq!(field.content_type().type_(), mime::TEXT);
                            assert_eq!(field.content_type().subtype(), mime::PLAIN);

                            match field.poll() {
                                Ok(Async::Ready(Some(chunk))) =>
                                    assert_eq!(chunk, "data"),
                                _ => unreachable!()
                            }
                            match field.poll() {
                                Ok(Async::Ready(None)) => (),
                                _ => unreachable!()
                            }
                        },
                        _ => unreachable!()
                    }
                }
                _ => unreachable!()
            }

            match multipart.poll() {
                Ok(Async::Ready(None)) => (),
                _ => unreachable!()
            }

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }
}
