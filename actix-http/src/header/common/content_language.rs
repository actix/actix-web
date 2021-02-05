use crate::header::{QualityItem, CONTENT_LANGUAGE};
use language_tags::LanguageTag;

header! {
    /// `Content-Language` header, defined in
    /// [RFC7231](https://tools.ietf.org/html/rfc7231#section-3.1.3.2)
    ///
    /// The `Content-Language` header field describes the natural language(s)
    /// of the intended audience for the representation.  Note that this
    /// might not be equivalent to all the languages used within the
    /// representation.
    ///
    /// # ABNF
    ///
    /// ```text
    /// Content-Language = 1#language-tag
    /// ```
    ///
    /// # Example values
    ///
    /// * `da`
    /// * `mi, en`
    ///
    /// # Examples
    ///
    /// ```
    /// use language_tags::langtag;
    /// use actix_http::Response;
    /// use actix_http::http::header::{ContentLanguage, qitem};
    ///
    /// let mut builder = Response::Ok();
    /// builder.insert_header(
    ///     ContentLanguage(vec![
    ///         qitem(langtag!(en)),
    ///     ])
    /// );
    /// ```
    ///
    /// ```
    /// use language_tags::langtag;
    /// use actix_http::Response;
    /// use actix_http::http::header::{ContentLanguage, qitem};
    ///
    /// let mut builder = Response::Ok();
    /// builder.insert_header(
    ///     ContentLanguage(vec![
    ///         qitem(langtag!(da)),
    ///         qitem(langtag!(en;;;GB)),
    ///     ])
    /// );
    /// ```
    (ContentLanguage, CONTENT_LANGUAGE) => (QualityItem<LanguageTag>)+

    test_content_language {
        test_header!(test1, vec![b"da"]);
        test_header!(test2, vec![b"mi, en"]);
    }
}
