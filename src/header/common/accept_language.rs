use language_tags::LanguageTag;
use header::{http, QualityItem};


header! {
    /// `Accept-Language` header, defined in
    /// [RFC7231](http://tools.ietf.org/html/rfc7231#section-5.3.5)
    ///
    /// The `Accept-Language` header field can be used by user agents to
    /// indicate the set of natural languages that are preferred in the
    /// response.
    ///
    /// # ABNF
    ///
    /// ```text
    /// Accept-Language = 1#( language-range [ weight ] )
    /// language-range  = <language-range, see [RFC4647], Section 2.1>
    /// ```
    ///
    /// # Example values
    /// * `da, en-gb;q=0.8, en;q=0.7`
    /// * `en-us;q=1.0, en;q=0.5, fr`
    ///
    /// # Examples
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # extern crate language_tags;
    /// use actix_web::httpcodes::HttpOk;
    /// use actix_web::header::{AcceptLanguage, LanguageTag, qitem};
    ///
    /// # fn main() {
    /// let mut builder = HttpOk.build();
    /// let mut langtag: LanguageTag = Default::default();
    /// langtag.language = Some("en".to_owned());
    /// langtag.region = Some("US".to_owned());
    /// builder.set(
    ///     AcceptLanguage(vec![
    ///         qitem(langtag),
    ///     ])
    /// );
    /// # }
    /// ```
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # #[macro_use] extern crate language_tags;
    /// use actix_web::httpcodes::HttpOk;
    /// use actix_web::header::{AcceptLanguage, QualityItem, q, qitem};
    /// #
    /// # fn main() {
    /// let mut builder = HttpOk.build();
    /// builder.set(
    ///     AcceptLanguage(vec![
    ///         qitem(langtag!(da)),
    ///         QualityItem::new(langtag!(en;;;GB), q(800)),
    ///         QualityItem::new(langtag!(en), q(700)),
    ///     ])
    /// );
    /// # }
    /// ```
    (AcceptLanguage, http::ACCEPT_LANGUAGE) => (QualityItem<LanguageTag>)+

    test_accept_language {
        // From the RFC
        test_header!(test1, vec![b"da, en-gb;q=0.8, en;q=0.7"]);
        // Own test
        test_header!(
            test2, vec![b"en-US, en; q=0.5, fr"],
            Some(AcceptLanguage(vec![
                qitem("en-US".parse().unwrap()),
                QualityItem::new("en".parse().unwrap(), q(500)),
                qitem("fr".parse().unwrap()),
        ])));
    }
}
