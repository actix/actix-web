use mime::Mime;

use super::CONTENT_TYPE;

crate::http::header::common_header! {
    /// `Content-Type` header, defined in [RFC 9110 §8.3].
    ///
    /// The `Content-Type` header field indicates the media type of the associated representation:
    /// either the representation enclosed in the message payload or the selected representation,
    /// as determined by the message semantics. The indicated media type defines both the data
    /// format and how that data is intended to be processed by a recipient, within the scope of the
    /// received message semantics, after any content codings indicated by Content-Encoding are
    /// decoded.
    ///
    /// Although the `mime` crate allows the mime options to be any slice, this crate forces the use
    /// of Vec. This is to make sure the same header can't have more than 1 type. If this is an
    /// issue, it's possible to implement `Header` on a custom struct.
    ///
    /// # ABNF
    ///
    /// ```plain
    /// Content-Type = media-type
    /// ```
    ///
    /// # Example Values
    ///
    /// - `text/html; charset=utf-8`
    /// - `application/json`
    ///
    /// # Examples
    ///
    /// ```
    /// use actix_web::{http::header::ContentType, HttpResponse};
    ///
    /// let res_json = HttpResponse::Ok()
    ///     .insert_header(ContentType::json());
    ///
    /// let res_html = HttpResponse::Ok()
    ///     .insert_header(ContentType(mime::TEXT_HTML));
    /// ```
    ///
    /// [RFC 9110 §8.3]: https://datatracker.ietf.org/doc/html/rfc9110#section-8.3
    (ContentType, CONTENT_TYPE) => [Mime]

    test_parse_and_format {
        crate::http::header::common_header_test!(
            test_text_html,
            [b"text/html"],
            Some(HeaderField(mime::TEXT_HTML)));
        crate::http::header::common_header_test!(
            test_image_star,
            [b"image/*"],
            Some(HeaderField(mime::IMAGE_STAR)));

    }
}

impl ContentType {
    /// Constructs a `Content-Type: application/json` header.
    #[inline]
    pub fn json() -> ContentType {
        ContentType(mime::APPLICATION_JSON)
    }

    /// Constructs a `Content-Type: text/plain; charset=utf-8` header.
    #[inline]
    pub fn plaintext() -> ContentType {
        ContentType(mime::TEXT_PLAIN_UTF_8)
    }

    /// Constructs a `Content-Type: text/html; charset=utf-8` header.
    #[inline]
    pub fn html() -> ContentType {
        ContentType(mime::TEXT_HTML_UTF_8)
    }

    /// Constructs a `Content-Type: text/xml` header.
    #[inline]
    pub fn xml() -> ContentType {
        ContentType(mime::TEXT_XML)
    }

    /// Constructs a `Content-Type: application/www-form-url-encoded` header.
    #[inline]
    pub fn form_url_encoded() -> ContentType {
        ContentType(mime::APPLICATION_WWW_FORM_URLENCODED)
    }

    /// Constructs a `Content-Type: image/jpeg` header.
    #[inline]
    pub fn jpeg() -> ContentType {
        ContentType(mime::IMAGE_JPEG)
    }

    /// Constructs a `Content-Type: image/png` header.
    #[inline]
    pub fn png() -> ContentType {
        ContentType(mime::IMAGE_PNG)
    }

    /// Constructs a `Content-Type: application/octet-stream` header.
    #[inline]
    pub fn octet_stream() -> ContentType {
        ContentType(mime::APPLICATION_OCTET_STREAM)
    }
}
