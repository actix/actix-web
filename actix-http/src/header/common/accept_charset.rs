use crate::header::{Charset, QualityItem, ACCEPT_CHARSET};

header! {
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
    /// ```rust
    /// # extern crate actix_http;
    /// use actix_http::Response;
    /// use actix_http::http::header::{AcceptCharset, Charset, qitem};
    ///
    /// # fn main() {
    /// let mut builder = Response::Ok();
    /// builder.set(
    ///     AcceptCharset(vec![qitem(Charset::Us_Ascii)])
    /// );
    /// # }
    /// ```
    /// ```rust
    /// # extern crate actix_http;
    /// use actix_http::Response;
    /// use actix_http::http::header::{AcceptCharset, Charset, q, QualityItem};
    ///
    /// # fn main() {
    /// let mut builder = Response::Ok();
    /// builder.set(
    ///     AcceptCharset(vec![
    ///         QualityItem::new(Charset::Us_Ascii, q(900)),
    ///         QualityItem::new(Charset::Iso_8859_10, q(200)),
    ///     ])
    /// );
    /// # }
    /// ```
    /// ```rust
    /// # extern crate actix_http;
    /// use actix_http::Response;
    /// use actix_http::http::header::{AcceptCharset, Charset, qitem};
    ///
    /// # fn main() {
    /// let mut builder = Response::Ok();
    /// builder.set(
    ///     AcceptCharset(vec![qitem(Charset::Ext("utf-8".to_owned()))])
    /// );
    /// # }
    /// ```
    (AcceptCharset, ACCEPT_CHARSET) => (QualityItem<Charset>)+

    test_accept_charset {
        // Test case from RFC
        test_header!(test1, vec![b"iso-8859-5, unicode-1-1;q=0.8"]);
    }
}
