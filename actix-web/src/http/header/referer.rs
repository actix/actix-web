use super::{Uri, REFERER};

crate::http::header::common_header! {
    /// `Referer` header, defined
    /// in [RFC 7231 §5.5.2](https://datatracker.ietf.org/doc/html/rfc7231#section-5.5.2)
    ///
    /// The "Referer" (sic) header field allows the user agent to specify a
    /// URI reference for the resource from which the target URI was obtained
    /// (i.e., the "referrer", though the field name is misspelled).
    ///
    /// # ABNF
    /// ```plain
    /// Referer = absolute-URI / partial-URI
    /// ```
    ///
    /// # Example Values
    /// * `http://www.example.org/hypertext/Overview.html`
    ///
    /// # Examples
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_http::Uri;
    /// use actix_web::http::header::Referer;
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     Referer("http://www.example.org".parse::<Uri>().unwrap())
    /// );
    /// ```
    (Referer, REFERER) => [Uri]

    test_parse_and_format {
        crate::http::header::common_header_test!(test1, [b"http://www.example.org/hypertext/Overview.html"]);
    }
}
