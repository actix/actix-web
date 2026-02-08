use super::{common_header, Charset, QualityItem, ACCEPT_CHARSET};

common_header! {
    /// `Accept-Charset` header, defined in [RFC 7231 §5.3.3].
    ///
    /// The `Accept-Charset` header field can be sent by a user agent to
    /// indicate what charsets are acceptable in textual response content.
    /// This field allows user agents capable of understanding more
    /// comprehensive or special-purpose charsets to signal that capability
    /// to an origin server that is capable of representing information in
    /// those charsets.
    ///
    /// # ABNF
    /// ```plain
    /// Accept-Charset = 1#( ( charset / "*" ) [ weight ] )
    /// ```
    ///
    /// # Example Values
    /// * `iso-8859-5, unicode-1-1;q=0.8`
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptCharset, Charset, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptCharset(vec![QualityItem::max(Charset::Us_Ascii)])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptCharset, Charset, q, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptCharset(vec![
    ///         QualityItem::new(Charset::Us_Ascii, q(0.9)),
    ///         QualityItem::new(Charset::Iso_8859_10, q(0.2)),
    ///     ])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptCharset, Charset, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptCharset(vec![QualityItem::max(Charset::Ext("utf-8".to_owned()))])
    /// );
    /// ```
    ///
    /// [RFC 7231 §5.3.3]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.3
    (AcceptCharset, ACCEPT_CHARSET) => (QualityItem<Charset>)*

    test_parse_and_format {
        // Test case from RFC
        common_header_test!(test1, [b"iso-8859-5, unicode-1-1;q=0.8"]);
    }
}
