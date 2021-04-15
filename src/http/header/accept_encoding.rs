use header::{Encoding, QualityItem};

header! {
    /// `Accept-Encoding` header, defined in
    /// [RFC7231](http://tools.ietf.org/html/rfc7231#section-5.3.4)
    ///
    /// The `Accept-Encoding` header field can be used by user agents to
    /// indicate what response content-codings are
    /// acceptable in the response.  An  `identity` token is used as a synonym
    /// for "no encoding" in order to communicate when no encoding is
    /// preferred.
    ///
    /// # ABNF
    ///
    /// ```text
    /// Accept-Encoding  = #( codings [ weight ] )
    /// codings          = content-coding / "identity" / "*"
    /// ```
    ///
    /// # Example values
    /// * `compress, gzip`
    /// * ``
    /// * `*`
    /// * `compress;q=0.5, gzip;q=1`
    /// * `gzip;q=1.0, identity; q=0.5, *;q=0`
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptEncoding, Encoding, qitem};
    ///
    /// let mut builder = HttpResponse::new();
    /// builder.insert_header(
    ///     AcceptEncoding(vec![qitem(Encoding::Chunked)])
    /// );
    /// ```
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptEncoding, Encoding, qitem};
    ///
    /// let mut builder = HttpResponse::new();
    /// builder.insert_header(
    ///     AcceptEncoding(vec![
    ///         qitem(Encoding::Chunked),
    ///         qitem(Encoding::Gzip),
    ///         qitem(Encoding::Deflate),
    ///     ])
    /// );
    /// ```
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptEncoding, Encoding, QualityItem, q, qitem};
    ///
    /// let mut builder = HttpResponse::new();
    /// builder.insert_header(
    ///     AcceptEncoding(vec![
    ///         qitem(Encoding::Chunked),
    ///         QualityItem::new(Encoding::Gzip, q(600)),
    ///         QualityItem::new(Encoding::EncodingExt("*".to_owned()), q(0)),
    ///     ])
    /// );
    /// ```
    (AcceptEncoding, "Accept-Encoding") => (QualityItem<Encoding>)*

    test_accept_encoding {
        // From the RFC
        crate::__common_header_test!(test1, vec![b"compress, gzip"]);
        crate::__common_header_test!(test2, vec![b""], Some(AcceptEncoding(vec![])));
        crate::__common_header_test!(test3, vec![b"*"]);
        // Note: Removed quality 1 from gzip
        crate::__common_header_test!(test4, vec![b"compress;q=0.5, gzip"]);
        // Note: Removed quality 1 from gzip
        crate::__common_header_test!(test5, vec![b"gzip, identity; q=0.5, *;q=0"]);
    }
}
