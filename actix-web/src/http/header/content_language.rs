use language_tags::LanguageTag;

use super::{common_header, QualityItem, CONTENT_LANGUAGE};

common_header! {
    /// `Content-Language` header, defined
    /// in [RFC 7231 §3.1.3.2](https://datatracker.ietf.org/doc/html/rfc7231#section-3.1.3.2)
    ///
    /// The `Content-Language` header field describes the natural language(s)
    /// of the intended audience for the representation.  Note that this
    /// might not be equivalent to all the languages used within the
    /// representation.
    ///
    /// # ABNF
    /// ```plain
    /// Content-Language = 1#language-tag
    /// ```
    ///
    /// # Example Values
    /// * `da`
    /// * `mi, en`
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{ContentLanguage, LanguageTag, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     ContentLanguage(vec![
    ///         QualityItem::max(LanguageTag::parse("en").unwrap()),
    ///     ])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{ContentLanguage, LanguageTag, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     ContentLanguage(vec![
    ///         QualityItem::max(LanguageTag::parse("da").unwrap()),
    ///         QualityItem::max(LanguageTag::parse("en-GB").unwrap()),
    ///     ])
    /// );
    /// ```
    (ContentLanguage, CONTENT_LANGUAGE) => (QualityItem<LanguageTag>)+

    test_parse_and_format {
        crate::http::header::common_header_test!(test1, [b"da"]);
        crate::http::header::common_header_test!(test2, [b"mi, en"]);
    }
}
