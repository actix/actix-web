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
use memchr::memmem;
use mime::Mime;

use crate::{
    error::Error,
    payload::{PayloadBuffer, PayloadRef},
    safety::Safety,
};

/// Error type returned from [`Field::bytes()`] when field data is larger than limit.
#[derive(Debug, Display, Error)]
#[display("Size limit exceeded while collecting field data")]
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

/// Search strategy selected once per field from its declared content type.
///
/// A match identifies only a candidate; delimiter validation is the same for both strategies.
#[derive(Clone, Copy)]
pub(crate) enum BoundarySearch {
    /// Search for CR in types that commonly contain LF-only text.
    Cr,

    /// Search for a dash followed by the boundary's first non-dash byte.
    Marker {
        needle: [u8; 2],
        /// Offset from the delimiter's first dash to the needle start.
        marker_offset: usize,
    },
}

impl BoundarySearch {
    /// Selects a search strategy from a boundary and the field's declared content type.
    ///
    /// CSV and SQL retain the marker search because they can contain frequent CRLF lines.
    /// Missing or unrecognized types also use the marker search. The type is only a hint;
    /// it does not change which delimiters the parser accepts.
    pub(crate) fn new(boundary: &str, content_type: Option<&Mime>) -> Self {
        match content_type {
            // CSV and SQL can contain many CRLF lines, so use the marker search to
            // avoid checking each CR as a possible delimiter.
            Some(ct) if matches!(ct.subtype().as_str(), "csv" | "sql") => {}

            // For text and structured documents, assume LF-only lines with few CR bytes,
            // so use the cheaper single-byte CR search.
            Some(ct)
                if ct.type_() == mime::TEXT
                    || matches!(ct.subtype().as_str(), "json" | "xml" | "yaml")
                    || ct
                        .suffix()
                        .is_some_and(|suffix| matches!(suffix.as_str(), "json" | "xml")) =>
            {
                return Self::Cr;
            }

            // Binary data can contain frequent incidental CR bytes, so use the marker
            // search. Also use marker search for unknown or missing types.
            None | Some(_) => {}
        }

        // Include the first non-dash boundary byte, skipping any leading dashes.
        let prefix_end = boundary.as_bytes().iter().position(|&byte| byte != b'-');
        let marker_offset = prefix_end.map_or(0, |idx| idx + 1);
        let needle = [
            b'-',
            prefix_end.map_or(b'-', |idx| boundary.as_bytes()[idx]),
        ];

        Self::Marker {
            needle,
            marker_offset,
        }
    }
}

pub(crate) struct InnerField {
    /// Payload is initialized as Some and is `take`n when the field stream finishes.
    payload: Option<PayloadRef>,

    /// Field boundary (without "--" prefix).
    boundary: String,
    boundary_search: BoundarySearch,

    /// True if request payload has been exhausted.
    eof: bool,

    /// Field data's stated size according to it's Content-Length header.
    length: Option<u64>,
}

impl InnerField {
    pub(crate) fn new_in_rc(
        payload: PayloadRef,
        boundary: String,
        boundary_search: BoundarySearch,
        headers: &HeaderMap,
    ) -> Result<Rc<RefCell<InnerField>>, PayloadError> {
        Self::new(payload, boundary, boundary_search, headers)
            .map(|this| Rc::new(RefCell::new(this)))
    }

    pub(crate) fn new(
        payload: PayloadRef,
        boundary: String,
        boundary_search: BoundarySearch,
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
            boundary_search,
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

    /// Reads field data up to the next multipart boundary when the field has no `Content-Length`.
    ///
    /// This is called to read the field contents, starting just after its header block and blank
    /// line. The multipart reader has already consumed the opening boundary and headers.
    ///
    /// Returns field data in chunks, then `None` at the next boundary. Leaves that boundary and its
    /// leading newline buffered so the multipart reader can start the next field or close the body.
    pub(crate) fn read_until_boundary(
        payload: &mut PayloadBuffer,
        boundary: &str,
        search: BoundarySearch,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        const CRLF_LEN: usize = b"\r\n".len();
        const CRLF_PREFIX_LEN: usize = b"\r\n--".len();

        let len = payload.buf.len();

        if len == 0 {
            // The field still needs a delimiter. An empty buffer is only temporary
            // while the input stream can supply more bytes.
            return if payload.eof {
                Poll::Ready(Some(Err(Error::Incomplete)))
            } else {
                Poll::Pending
            };
        }

        // Check for a delimiter left at the buffer start by a previous read.
        // Leave it for the multipart reader to consume after the field ends.
        // Only CRLF introduces a delimiter; a bare CR followed by --boundary is field data.
        if payload.buf.starts_with(b"\r\n--") {
            let b_len = CRLF_PREFIX_LEN;
            let b_size = boundary.len() + b_len;
            let available = len - b_len;
            let check_len = cmp::min(available, boundary.len());

            // A matching prefix must stay buffered; a mismatch is ordinary field data.
            if payload.buf[b_len..b_len + check_len] == boundary.as_bytes()[..check_len] {
                match (len >= b_size, payload.eof) {
                    // The full boundary matches, so this field has ended.
                    (true, _) => return Poll::Ready(None),

                    // The next chunk can complete this boundary.
                    (false, false) => return Poll::Pending,

                    // EOF cannot complete the partial boundary.
                    (false, true) => return Poll::Ready(Some(Err(Error::Incomplete))),
                }
            }
        }

        // `pos` is the next search position in this buffer. Advancing it skips rejected
        // candidates; it does not remove data. Only the return paths consume field bytes.
        let mut pos = 0;

        loop {
            let candidate_boundary_start = match search {
                BoundarySearch::Cr => {
                    // A CR match already points to the start of a possible delimiter.
                    memchr::memchr(b'\r', &payload.buf[pos..]).map(|offset| pos + offset)
                }
                BoundarySearch::Marker {
                    needle,
                    marker_offset,
                } => {
                    // Search from `pos` for the cached two-byte needle. The result is relative
                    // to `pos` and points inside a possible delimiter, not at its leading CR.
                    let needle_start = memmem::find(&payload.buf[pos..], &needle);

                    let candidate_boundary_start = needle_start
                        .and_then(|idx| {
                            // Walk back to the first dash of the delimiter's `--` prefix. For
                            // boundary `--abc`, the bytes must be `\r\n----abc`: the needle `-a`
                            // starts at the last dash, three bytes after the first dash.
                            let hyphen = (pos + idx).checked_sub(marker_offset)?;

                            // Every byte from that first dash through the needle's dash must be
                            // a dash. Otherwise this needle match belongs to the field contents.
                            if !payload.buf[hyphen..=pos + idx]
                                .iter()
                                .all(|&byte| byte == b'-')
                            {
                                return None;
                            }

                            // A delimiter also needs CRLF before
                            // its dashes. Return the CR position so the newline stays buffered too.
                            if hyphen >= CRLF_LEN
                                && &payload.buf[hyphen - CRLF_LEN..hyphen] == b"\r\n"
                            {
                                Some(hyphen - CRLF_LEN)
                            } else {
                                None
                            }
                        })
                        // Do not reconsider a delimiter start that an earlier iteration rejected.
                        .filter(|&idx| idx >= pos);

                    if let Some(idx) = needle_start {
                        if candidate_boundary_start.is_none() {
                            // This match is field data. Advance one byte past its start so an
                            // overlapping needle, such as the `--` fallback, can still match.
                            pos += idx + 1;
                            continue;
                        }
                    }

                    candidate_boundary_start.or_else(|| {
                        // No boundary candidate was found above. Check for any partial delimiter at
                        // the buffer end so we can safely yield the preceding bytes as field data.
                        //
                        // Even a 2-byte needle may be split across chunks. Search the tail for its
                        // preceding CR, allowing for CRLF and any leading dashes, and retain bytes
                        // from there.

                        const TAIL_LEN: usize = CRLF_LEN + b"-".len();

                        let tail_start = len.saturating_sub(TAIL_LEN + marker_offset).max(pos);

                        memchr::memchr(b'\r', &payload.buf[tail_start..])
                            .map(|cr_offset| tail_start + cr_offset)
                    })
                }
            };

            return if let Some(idx) = candidate_boundary_start {
                // `idx` points at a CR.

                if idx + CRLF_PREFIX_LEN > len {
                    if idx > 0 {
                        // Fewer than `CRLF_PREFIX_LEN` bytes remaining so we cannot finish
                        // checking the delimiter prefix. Emit only preceding field data.
                        Poll::Ready(Some(Ok(payload.buf.split_to(idx).freeze())))
                    } else if payload.eof {
                        Poll::Ready(Some(Err(Error::Incomplete)))
                    } else {
                        Poll::Pending
                    }
                } else {
                    fn delimiter_starts_at(pl: &PayloadBuffer, idx: usize) -> bool {
                        let buf = &pl.buf[idx..];
                        buf.starts_with(b"\r\n--")
                    }

                    // Check the delimiter prefix at `idx`. This also rejects ordinary
                    // CR bytes found by the trailing-byte search above.
                    if delimiter_starts_at(payload, idx) {
                        if idx > 0 {
                            // Emit field data before the CR. On the next read, this
                            // candidate is at offset zero for the full boundary check.
                            Poll::Ready(Some(Ok(payload.buf.split_to(idx).freeze())))
                        } else {
                            // The full boundary check at entry already rejected this
                            // offset-zero candidate. Keep scanning after its CR.
                            pos = idx + 1;
                            continue;
                        }
                    } else {
                        // This CR is field data. Continue searching after it.
                        pos = idx + 1;
                        continue;
                    }
                }
            } else {
                // No delimiter candidate or partial prefix remains. Therefore,
                // all buffered bytes belong to this field and can be yielded.
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
                Self::read_until_boundary(&mut payload, &self.boundary, self.boundary_search)
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
    use actix_http::h1;
    use futures_util::{stream, FutureExt as _, StreamExt as _};

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

    #[tokio::test(flavor = "local")]
    async fn bytes_unlimited() {
        let (body, headers) = create_double_request_with_header();

        let mut multipart = Multipart::new(&headers, stream::iter([Ok(body)]));

        let field = multipart
            .next()
            .await
            .expect("Multipart should have two fields")
            .expect("Multipart body should be well formatted")
            .bytes(usize::MAX)
            .await
            .expect("Field data should not be size limited")
            .expect("Reading field data should not error");
        assert_eq!(field, "one+one+one");

        let field = multipart
            .next()
            .await
            .expect("Multipart should have two fields")
            .expect("Multipart body should be well formatted")
            .bytes(usize::MAX)
            .await
            .expect("Field data should not be size limited")
            .expect("Reading field data should not error");
        assert_eq!(field, "two+two+two");
    }

    #[tokio::test(flavor = "local")]
    async fn bytes_limited() {
        let (body, headers) = create_double_request_with_header();

        let mut multipart = Multipart::new(&headers, stream::iter([Ok(body)]));

        multipart
            .next()
            .await
            .expect("Multipart should have two fields")
            .expect("Multipart body should be well formatted")
            .bytes(8) // smaller than data size
            .await
            .expect_err("field data should be size limited");

        // next field still readable
        let field = multipart
            .next()
            .await
            .expect("Multipart should have two fields")
            .expect("Multipart body should be well formatted")
            .bytes(usize::MAX)
            .await
            .expect("Field data should not be size limited")
            .expect("Reading field data should not error");
        assert_eq!(field, "two+two+two");
    }

    #[tokio::test(flavor = "local")]
    async fn boundary_marker_split_across_chunks() {
        let (body, headers) = create_double_request_with_header();
        let boundary_start = memchr::memmem::find_iter(&body, b"\r\n--")
            .nth(1)
            .expect("Body should contain a boundary between its fields");
        let boundary_marker_end = boundary_start + 4;

        let (mut tx, rx) = h1::Payload::create(false);
        tx.feed_data(body.slice(..boundary_start));
        tx.feed_data(body.slice(boundary_start..boundary_marker_end));

        let mut multipart = Multipart::new(&headers, rx);

        let mut field = multipart
            .next()
            .await
            .expect("Multipart should have two fields")
            .expect("Multipart body should be well formatted");
        assert_eq!(field.next().await.unwrap().unwrap(), "one+one+one");
        let next = field.next().now_or_never();
        assert!(
            next.is_none(),
            "partial boundary marker should not be emitted as field data: {next:?}",
        );

        tx.feed_data(body.slice(boundary_marker_end..));
        assert!(field.next().await.is_none());
        drop(field);

        let field = multipart
            .next()
            .await
            .expect("Multipart should have two fields")
            .expect("Multipart body should be well formatted")
            .bytes(usize::MAX)
            .await
            .expect("Field data should not be size limited")
            .expect("Reading field data should not error");
        assert_eq!(field, "two+two+two");
    }

    #[tokio::test(flavor = "local")]
    async fn bare_cr_before_boundary_is_field_data() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("multipart/form-data; boundary=abc"),
        );

        let body = Bytes::from_static(
            b"--abc\r\nContent-Disposition: form-data; name=\"field\"\r\n\r\n\
              one\r--abc two\r\n--abc--\r\n",
        );
        let bare_cr = memchr::memmem::find(&body, b"\r--abc two").unwrap();

        for split in [false, true] {
            let (mut tx, rx) = h1::Payload::create(false);
            tx.feed_data(if split {
                body.slice(..bare_cr)
            } else {
                body.clone()
            });

            let mut multipart = Multipart::new(&headers, rx);

            let mut field = multipart
                .next()
                .await
                .expect("Multipart should have one field")
                .expect("Multipart body should be well formatted");

            if split {
                assert_eq!(field.next().await.unwrap().unwrap(), "one");
                assert!(field.next().now_or_never().is_none());

                // Consume the first chunk before the bare CR can enter the parser buffer.
                tx.feed_data(body.slice(bare_cr..));
            }

            tx.feed_eof();

            let bytes = field
                .bytes(usize::MAX)
                .await
                .expect("Field data should not be size limited")
                .expect("Reading field data should not error");
            assert_eq!(
                bytes,
                if split {
                    "\r--abc two"
                } else {
                    "one\r--abc two"
                }
            );

            drop(field);
            assert!(multipart.next().await.is_none());
        }
    }

    #[test]
    fn mismatching_boundary_prefix_is_yielded_without_more_input() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("multipart/form-data; boundary=abc"),
        );

        let (mut tx, rx) = h1::Payload::create(false);
        tx.feed_data(Bytes::from_static(
            b"--abc\r\nContent-Disposition: form-data; name=\"field\"\r\n\r\n\r\n--ax",
        ));

        let mut multipart = Multipart::new(&headers, rx);
        let mut field = multipart
            .next()
            .now_or_never()
            .expect("Field headers should be ready")
            .expect("Multipart should contain a field")
            .expect("Field headers should be valid");

        let next = field.next().now_or_never();
        assert!(
            matches!(&next, Some(Some(Ok(bytes))) if bytes.as_ref() == b"\r\n--ax"),
            "a mismatching boundary prefix should be yielded without more input: {next:?}",
        );

        tx.feed_eof();

        let next = field.next().now_or_never();
        assert!(
            matches!(next, Some(Some(Err(crate::error::Error::Incomplete)))),
            "EOF without a closing boundary should report an incomplete message: {next:?}",
        );
    }

    #[test]
    fn mismatching_boundary_prefix_is_yielded_before_eof_incomplete() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("multipart/form-data; boundary=abc"),
        );

        let (mut tx, rx) = h1::Payload::create(false);
        tx.feed_data(Bytes::from_static(
            b"--abc\r\nContent-Disposition: form-data; name=\"field\"\r\n\r\n\r\n--ax",
        ));
        tx.feed_eof();

        let mut multipart = Multipart::new(&headers, rx);
        let mut field = multipart
            .next()
            .now_or_never()
            .expect("Field headers should be ready")
            .expect("Multipart should contain a field")
            .expect("Field headers should be valid");

        let next = field.next().now_or_never();
        assert!(
            matches!(&next, Some(Some(Ok(bytes))) if bytes.as_ref() == b"\r\n--ax"),
            "a mismatching boundary prefix should be yielded before Incomplete: {next:?}",
        );

        let next = field.next().now_or_never();
        assert!(
            matches!(next, Some(Some(Err(crate::error::Error::Incomplete)))),
            "EOF after yielding mismatch should report an incomplete message: {next:?}",
        );
    }

    #[tokio::test(flavor = "local")]
    async fn marker_candidates_and_all_chunk_splits_preserve_field_data() {
        for content_type in [
            "",
            "Content-Type: text/plain\r\n",
            "Content-Type: application/json\r\n",
            "Content-Type: application/vnd.example+json\r\n",
            "Content-Type: image/svg+xml\r\n",
            "Content-Type: text/csv\r\n",
            "Content-Type: application/sql\r\n",
            "Content-Type: application/octet-stream\r\n",
        ] {
            for boundary in ["boundary", "-boundary", "----WebKitBoundary", "---", "b"] {
                let data = format!(
                    "--leading\r\n--wrong\r--{boundary} text\r\n---overlap\r\nplain\rtext--trailing"
                );
                let body = Bytes::from(format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"\r\n{content_type}\r\n{data}\r\n--{boundary}--\r\n"
                ));
                let mut headers = HeaderMap::new();
                headers.insert(
                    header::CONTENT_TYPE,
                    header::HeaderValue::from_str(&format!(
                        "multipart/form-data; boundary={boundary}"
                    ))
                    .unwrap(),
                );

                for split in 1..body.len() {
                    let mut chunks = [body.slice(..split), body.slice(split..)].into_iter();
                    let mut pending = false;
                    let input = stream::poll_fn(move |cx| {
                        pending = !pending;
                        if pending {
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        } else {
                            Poll::Ready(chunks.next().map(Ok))
                        }
                    });
                    let mut multipart = Multipart::new(&headers, input);
                    let mut field = multipart.next().await.unwrap().unwrap();
                    let actual = field.bytes(usize::MAX).await.unwrap().unwrap();

                    assert_eq!(
                        actual,
                        data.as_str(),
                        "{content_type:?}, boundary {boundary:?}, split at {split}"
                    );
                    drop(field);
                    assert!(multipart.next().await.is_none());
                }
            }
        }
    }
}
