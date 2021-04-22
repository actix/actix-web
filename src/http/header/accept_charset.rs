use super::{Charset, QualityItem, ACCEPT_CHARSET};

crate::__define_common_header! {
    /// `Accept-Charset` header, defined in
    /// [RFC7231](http://tools.ietf.org/html/rfc7231#section-5.3.3)
    ///
    /// The `Accept-Charset` header field can be sent by a user agent to
    /// indicate what charsets are acceptable in textual response content.
    /// This field allows user agents capable of understanding more
    /// comprehensive or special-purpose charsets to signal that capability
    /// to an origin server that is capable of representing information in
    /// those charsets.
    ///
    /// # ABNF
    ///
    /// ```text
    /// Accept-Charset = 1#( ( charset / "*" ) [ weight ] )
    /// ```
    ///
    /// # Example values
    /// * `iso-8859-5, unicode-1-1;q=0.8`
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptCharset, Charset, qitem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptCharset(vec![qitem(Charset::Us_Ascii)])
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
    ///         QualityItem::new(Charset::Us_Ascii, q(900)),
    ///         QualityItem::new(Charset::Iso_8859_10, q(200)),
    ///     ])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptCharset, Charset, qitem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptCharset(vec![qitem(Charset::Ext("utf-8".to_owned()))])
    /// );
    /// ```
    (AcceptCharset, ACCEPT_CHARSET) => (QualityItem<Charset>)+

    test_accept_charset {
        // Test case from RFC
        crate::__common_header_test!(test1, vec![b"iso-8859-5, unicode-1-1;q=0.8"]);
    }
}
