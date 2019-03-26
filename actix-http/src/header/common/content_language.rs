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
    /// ```rust
    /// # extern crate actix_http;
    /// # #[macro_use] extern crate language_tags;
    /// use actix_http::Response;
    /// # use actix_http::http::header::{ContentLanguage, qitem};
    /// #
    /// # fn main() {
    /// let mut builder = Response::Ok();
    /// builder.set(
    ///     ContentLanguage(vec![
    ///         qitem(langtag!(en)),
    ///     ])
    /// );
    /// # }
    /// ```
    ///
    /// ```rust
    /// # extern crate actix_http;
    /// # #[macro_use] extern crate language_tags;
    /// use actix_http::Response;
    /// # use actix_http::http::header::{ContentLanguage, qitem};
    /// #
    /// # fn main() {
    ///
    /// let mut builder = Response::Ok();
    /// builder.set(
    ///     ContentLanguage(vec![
    ///         qitem(langtag!(da)),
    ///         qitem(langtag!(en;;;GB)),
    ///     ])
    /// );
    /// # }
    /// ```
    (ContentLanguage, CONTENT_LANGUAGE) => (QualityItem<LanguageTag>)+

    test_content_language {
        test_header!(test1, vec![b"da"]);
        test_header!(test2, vec![b"mi, en"]);
    }
}
