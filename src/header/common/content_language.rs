use language_tags::LanguageTag;
use header::{http, QualityItem};


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
    /// # extern crate actix_web;
    /// # #[macro_use] extern crate language_tags;
    /// use actix_web::httpcodes::HttpOk;
    /// # use actix_web::header::{ContentLanguage, qitem};
    /// # 
    /// # fn main() {
    /// let mut builder = HttpOk.build();
    /// builder.set(
    ///     ContentLanguage(vec![
    ///         qitem(langtag!(en)),
    ///     ])
    /// );
    /// # }
    /// ```
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # #[macro_use] extern crate language_tags;
    /// use actix_web::httpcodes::HttpOk;
    /// # use actix_web::header::{ContentLanguage, qitem};
    /// #
    /// # fn main() {
    /// 
    /// let mut builder = HttpOk.build();
    /// builder.set(
    ///     ContentLanguage(vec![
    ///         qitem(langtag!(da)),
    ///         qitem(langtag!(en;;;GB)),
    ///     ])
    /// );
    /// # }
    /// ```
    (ContentLanguage, http::CONTENT_LANGUAGE) => (QualityItem<LanguageTag>)+

    test_content_language {
        test_header!(test1, vec![b"da"]);
        test_header!(test2, vec![b"mi, en"]);
    }
}
