use std::{
    cell::RefCell,
    cmp, fmt,
    future::poll_fn,
    mem,
    pin::Pin,
    rc::Rc,
    task::{ready, Context, Poll},
};

use actix_web::{
    error::PayloadError,
    http::header::{self, ContentDisposition, HeaderMap},
    web::{Bytes, BytesMut},
};
use derive_more::{Display, Error};
use futures_core::Stream;
use mime::Mime;

use crate::{
    error::Error,
    payload::{PayloadBuffer, PayloadRef},
    safety::Safety,
};

/// Error type returned from [`Field::bytes()`] when field data is larger than limit.
#[derive(Debug, Display, Error)]
#[display("size limit exceeded while collecting field data")]
#[non_exhaustive]
pub struct LimitExceeded;

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

    /// Collects the raw field data, up to `limit` bytes.
    ///
    /// # Errors
    ///
    /// Any errors produced by the data stream are returned as `Ok(Err(Error))` immediately.
    ///
    /// If the buffered data size would exceed `limit`, an `Err(LimitExceeded)` is returned. Note
    /// that, in this case, the full data stream is exhausted before returning the error so that
    /// subsequent fields can still be read. To better defend against malicious/infinite requests,
    /// it is advisable to also put a timeout on this call.
    pub async fn bytes(&mut self, limit: usize) -> Result<Result<Bytes, Error>, LimitExceeded> {
        /// Sensible default (2kB) for initial, bounded allocation when collecting body bytes.
        const INITIAL_ALLOC_BYTES: usize = 2 * 1024;

        let mut exceeded_limit = false;
        let mut buf = BytesMut::with_capacity(INITIAL_ALLOC_BYTES);

        let mut field = Pin::new(self);

        match poll_fn(|cx| loop {
            match ready!(field.as_mut().poll_next(cx)) {
                // if already over limit, discard chunk to advance multipart request
                Some(Ok(_chunk)) if exceeded_limit => {}

                // if limit is exceeded set flag to true and continue
                Some(Ok(chunk)) if buf.len() + chunk.len() > limit => {
                    exceeded_limit = true;
                    // eagerly de-allocate field data buffer
                    let _ = mem::take(&mut buf);
                }

                Some(Ok(chunk)) => buf.extend_from_slice(&chunk),

                None => return Poll::Ready(Ok(())),
                Some(Err(err)) => return Poll::Ready(Err(err)),
            }
        })
        .await
        {
            // propagate error returned from body poll
            Err(err) => Ok(Err(err)),

            // limit was exceeded while reading body
            Ok(()) if exceeded_limit => Err(LimitExceeded),

            // otherwise return body buffer
            Ok(()) => Ok(Ok(buf.freeze())),
        }
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

    /// Field boundary (without "--" prefix).
    boundary: String,

    /// True if request payload has been exhausted.
    eof: bool,

    /// Field data's stated size according to it's Content-Length header.
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
            let b_len = if payload.buf.starts_with(b"\r\n") && &payload.buf[2..4] == b"--" {
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

        let Some(mut payload) = self
            .payload
            .as_ref()
            .expect("Field should not be polled after completion")
            .get_mut(safety)
        else {
            return Poll::Pending;
        };

        if !self.eof {
            let res = if let Some(ref mut len) = self.length {
                Self::read_len(&mut payload, len)
            } else {
                Self::read_stream(&mut payload, &self.boundary)
            };

            match ready!(res) {
                Some(Ok(bytes)) => return Poll::Ready(Some(Ok(bytes))),
                Some(Err(err)) => return Poll::Ready(Some(Err(err))),
                None => self.eof = true,
            }
        }

        let result = match payload.readline() {
            Ok(None) => Poll::Pending,
            Ok(Some(line)) => {
                if line.as_ref() != b"\r\n" {
                    log::warn!("multipart field did not read all the data or it is malformed");
                }
                Poll::Ready(None)
            }
            Err(err) => Poll::Ready(Some(Err(err))),
        };

        drop(payload);

        if let Poll::Ready(None) = result {
            // drop payload buffer and make future un-poll-able
            let _ = self.payload.take();
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use futures_util::{stream, StreamExt as _};

    use super::*;
    use crate::Multipart;

    // TODO: use test utility when multi-file support is introduced
    fn create_double_request_with_header() -> (Bytes, HeaderMap) {
        let bytes = Bytes::from(
            "testasdadsad\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"fn.txt\"\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             \r\n\
             one+one+one\r\n\
             --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"fn.txt\"\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             \r\n\
             two+two+two\r\n\
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
    async fn bytes_unlimited() {
        let (body, headers) = create_double_request_with_header();

        let mut multipart = Multipart::new(&headers, stream::iter([Ok(body)]));

        let field = multipart
            .next()
            .await
            .expect("multipart should have two fields")
            .expect("multipart body should be well formatted")
            .bytes(usize::MAX)
            .await
            .expect("field data should not be size limited")
            .expect("reading field data should not error");
        assert_eq!(field, "one+one+one");

        let field = multipart
            .next()
            .await
            .expect("multipart should have two fields")
            .expect("multipart body should be well formatted")
            .bytes(usize::MAX)
            .await
            .expect("field data should not be size limited")
            .expect("reading field data should not error");
        assert_eq!(field, "two+two+two");
    }

    #[actix_rt::test]
    async fn bytes_limited() {
        let (body, headers) = create_double_request_with_header();

        let mut multipart = Multipart::new(&headers, stream::iter([Ok(body)]));

        multipart
            .next()
            .await
            .expect("multipart should have two fields")
            .expect("multipart body should be well formatted")
            .bytes(8) // smaller than data size
            .await
            .expect_err("field data should be size limited");

        // next field still readable
        let field = multipart
            .next()
            .await
            .expect("multipart should have two fields")
            .expect("multipart body should be well formatted")
            .bytes(usize::MAX)
            .await
            .expect("field data should not be size limited")
            .expect("reading field data should not error");
        assert_eq!(field, "two+two+two");
    }
}
