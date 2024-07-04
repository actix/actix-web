use std::{
    cell::RefCell,
    cmp, fmt,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_web::{
    error::PayloadError,
    http::header::{self, ContentDisposition, HeaderMap},
    web::Bytes,
};
use futures_core::stream::Stream;
use mime::Mime;

use crate::{
    error::Error,
    payload::{PayloadBuffer, PayloadRef},
    safety::Safety,
};

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
    pub(crate) fn new(
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
    type Item = Result<Bytes, Error>;

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
            return Poll::Ready(Some(Err(Error::NotConsumed)));
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

pub(crate) struct InnerField {
    /// Payload is initialized as Some and is `take`n when the field stream finishes.
    payload: Option<PayloadRef>,
    boundary: String,
    eof: bool,
    length: Option<u64>,
}

impl InnerField {
    pub(crate) fn new_in_rc(
        payload: PayloadRef,
        boundary: String,
        headers: &HeaderMap,
    ) -> Result<Rc<RefCell<InnerField>>, PayloadError> {
        Self::new(payload, boundary, headers).map(|this| Rc::new(RefCell::new(this)))
    }

    pub(crate) fn new(
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
    ///
    /// The body part must has `Content-Length` header with proper value.
    pub(crate) fn read_len(
        payload: &mut PayloadBuffer,
        size: &mut u64,
    ) -> Poll<Option<Result<Bytes, Error>>> {
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
                        Poll::Ready(Some(Err(Error::Incomplete)))
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
    pub(crate) fn read_stream(
        payload: &mut PayloadBuffer,
        boundary: &str,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        let mut pos = 0;

        let len = payload.buf.len();
        if len == 0 {
            return if payload.eof {
                Poll::Ready(Some(Err(Error::Incomplete)))
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

    pub(crate) fn poll(&mut self, safety: &Safety) -> Poll<Option<Result<Bytes, Error>>> {
        if self.payload.is_none() {
            return Poll::Ready(None);
        }

        let result = if let Some(mut payload) = self
            .payload
            .as_ref()
            .expect("Field should not be polled after completion")
            .get_mut(safety)
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
