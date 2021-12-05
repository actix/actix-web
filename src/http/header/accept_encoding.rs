use actix_http::header::QualityItem;

use super::{common_header, Encoding};
use crate::http::header;

common_header! {
    /// `Accept-Encoding` header, defined
    /// in [RFC 7231](https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.4)
    ///
    /// The `Accept-Encoding` header field can be used by user agents to
    /// indicate what response content-codings are
    /// acceptable in the response.  An  `identity` token is used as a synonym
    /// for "no encoding" in order to communicate when no encoding is
    /// preferred.
    ///
    /// # ABNF
    /// ```plain
    /// Accept-Encoding  = #( codings [ weight ] )
    /// codings          = content-coding / "identity" / "*"
    /// ```
    ///
    /// # Example Values
    /// * `compress, gzip`
    /// * ``
    /// * `*`
    /// * `compress;q=0.5, gzip;q=1`
    /// * `gzip;q=1.0, identity; q=0.5, *;q=0`
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptEncoding, Encoding, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptEncoding(vec![QualityItem::max(Encoding::Chunked)])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptEncoding, Encoding, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptEncoding(vec![
    ///         QualityItem::max(Encoding::Chunked),
    ///         QualityItem::max(Encoding::Gzip),
    ///         QualityItem::max(Encoding::Deflate),
    ///     ])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptEncoding, Encoding, QualityItem, q};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptEncoding(vec![
    ///         QualityItem::max(Encoding::Chunked),
    ///         QualityItem::new(Encoding::Gzip, q(600)),
    ///         QualityItem::min(Encoding::EncodingExt("*".to_owned())),
    ///     ])
    /// );
    /// ```
    (AcceptEncoding, header::ACCEPT_ENCODING) => (QualityItem<Encoding>)*

    test_parse_and_format {
        // From the RFC
        common_header_test!(test1, vec![b"compress, gzip"]);
        common_header_test!(test2, vec![b""], Some(AcceptEncoding(vec![])));
        common_header_test!(test3, vec![b"*"]);

        // Note: Removed quality 1 from gzip
        common_header_test!(test4, vec![b"compress;q=0.5, gzip"]);

        // Note: Removed quality 1 from gzip
        common_header_test!(test5, vec![b"gzip, identity; q=0.5, *;q=0"]);
    }
}
