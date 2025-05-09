//! Multipart testing utilities.

use actix_web::{
    http::header::{self, HeaderMap},
    web::{BufMut as _, Bytes, BytesMut},
};
use mime::Mime;
use rand::distr::{Alphanumeric, SampleString as _};

const CRLF: &[u8] = b"\r\n";
const CRLF_CRLF: &[u8] = b"\r\n\r\n";
const HYPHENS: &[u8] = b"--";
const BOUNDARY_PREFIX: &str = "------------------------";

/// Constructs a `multipart/form-data` payload from bytes and metadata.
///
/// Returned header map can be extended or merged with existing headers.
///
/// Multipart boundary used is a random alphanumeric string.
///
/// # Examples
///
/// ```
/// use actix_multipart::test::create_form_data_payload_and_headers;
/// use actix_web::{test::TestRequest, web::Bytes};
/// use memchr::memmem::find;
///
/// let (body, headers) = create_form_data_payload_and_headers(
///     "foo",
///     Some("lorem.txt".to_owned()),
///     Some(mime::TEXT_PLAIN_UTF_8),
///     Bytes::from_static(b"Lorem ipsum."),
/// );
///
/// assert!(find(&body, b"foo").is_some());
/// assert!(find(&body, b"lorem.txt").is_some());
/// assert!(find(&body, b"text/plain; charset=utf-8").is_some());
/// assert!(find(&body, b"Lorem ipsum.").is_some());
///
/// let req = TestRequest::default();
///
/// // merge header map into existing test request and set multipart body
/// let req = headers
///     .into_iter()
///     .fold(req, |req, hdr| req.insert_header(hdr))
///     .set_payload(body)
///     .to_http_request();
///
/// assert!(
///     req.headers()
///         .get("content-type")
///         .unwrap()
///         .to_str()
///         .unwrap()
///         .starts_with("multipart/form-data; boundary=\"")
/// );
/// ```
pub fn create_form_data_payload_and_headers(
    name: &str,
    filename: Option<String>,
    content_type: Option<Mime>,
    file: Bytes,
) -> (Bytes, HeaderMap) {
    let boundary = Alphanumeric.sample_string(&mut rand::rng(), 32);

    create_form_data_payload_and_headers_with_boundary(
        &boundary,
        name,
        filename,
        content_type,
        file,
    )
}

/// Constructs a `multipart/form-data` payload from bytes and metadata with a fixed boundary.
///
/// See [`create_form_data_payload_and_headers`] for more details.
pub fn create_form_data_payload_and_headers_with_boundary(
    boundary: &str,
    name: &str,
    filename: Option<String>,
    content_type: Option<Mime>,
    file: Bytes,
) -> (Bytes, HeaderMap) {
    let mut buf = BytesMut::with_capacity(file.len() + 128);

    let boundary_str = [BOUNDARY_PREFIX, boundary].concat();
    let boundary = boundary_str.as_bytes();

    buf.put(HYPHENS);
    buf.put(boundary);
    buf.put(CRLF);

    buf.put(format!("Content-Disposition: form-data; name=\"{name}\"").as_bytes());
    if let Some(filename) = filename {
        buf.put(format!("; filename=\"{filename}\"").as_bytes());
    }
    buf.put(CRLF);

    if let Some(ct) = content_type {
        buf.put(format!("Content-Type: {ct}").as_bytes());
        buf.put(CRLF);
    }

    buf.put(format!("Content-Length: {}", file.len()).as_bytes());
    buf.put(CRLF_CRLF);

    buf.put(file);
    buf.put(CRLF);

    buf.put(HYPHENS);
    buf.put(boundary);
    buf.put(HYPHENS);
    buf.put(CRLF);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        format!("multipart/form-data; boundary=\"{boundary_str}\"")
            .parse()
            .unwrap(),
    );

    (buf.freeze(), headers)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use futures_util::stream;

    use super::*;

    fn find_boundary(headers: &HeaderMap) -> String {
        headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .parse::<mime::Mime>()
            .unwrap()
            .get_param(mime::BOUNDARY)
            .unwrap()
            .as_str()
            .to_owned()
    }

    #[test]
    fn wire_format() {
        let (pl, headers) = create_form_data_payload_and_headers_with_boundary(
            "qWeRtYuIoP",
            "foo",
            None,
            None,
            Bytes::from_static(b"Lorem ipsum dolor\nsit ame."),
        );

        assert_eq!(
            find_boundary(&headers),
            "------------------------qWeRtYuIoP",
        );

        assert_eq!(
            std::str::from_utf8(&pl).unwrap(),
            "--------------------------qWeRtYuIoP\r\n\
            Content-Disposition: form-data; name=\"foo\"\r\n\
            Content-Length: 26\r\n\
            \r\n\
            Lorem ipsum dolor\n\
            sit ame.\r\n\
            --------------------------qWeRtYuIoP--\r\n",
        );

        let (pl, _headers) = create_form_data_payload_and_headers_with_boundary(
            "qWeRtYuIoP",
            "foo",
            Some("Lorem.txt".to_owned()),
            Some(mime::TEXT_PLAIN_UTF_8),
            Bytes::from_static(b"Lorem ipsum dolor\nsit ame."),
        );

        assert_eq!(
            std::str::from_utf8(&pl).unwrap(),
            "--------------------------qWeRtYuIoP\r\n\
            Content-Disposition: form-data; name=\"foo\"; filename=\"Lorem.txt\"\r\n\
            Content-Type: text/plain; charset=utf-8\r\n\
            Content-Length: 26\r\n\
            \r\n\
            Lorem ipsum dolor\n\
            sit ame.\r\n\
            --------------------------qWeRtYuIoP--\r\n",
        );
    }

    /// Test using an external library to prevent the two-wrongs-make-a-right class of errors.
    #[actix_web::test]
    async fn ecosystem_compat() {
        let (pl, headers) = create_form_data_payload_and_headers(
            "foo",
            None,
            None,
            Bytes::from_static(b"Lorem ipsum dolor\nsit ame."),
        );

        let boundary = find_boundary(&headers);

        let pl = stream::once(async { Ok::<_, Infallible>(pl) });

        let mut form = multer::Multipart::new(pl, boundary);
        let field = form.next_field().await.unwrap().unwrap();
        assert_eq!(field.name().unwrap(), "foo");
        assert_eq!(field.file_name(), None);
        assert_eq!(field.content_type(), None);
        assert!(field.bytes().await.unwrap().starts_with(b"Lorem"));
    }
}
