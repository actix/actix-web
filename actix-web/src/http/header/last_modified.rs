use super::{HttpDate, LAST_MODIFIED};

crate::http::header::common_header! {
    /// `Last-Modified` header, defined
    /// in [RFC 7232 §2.2](https://datatracker.ietf.org/doc/html/rfc7232#section-2.2)
    ///
    /// The `Last-Modified` header field in a response provides a timestamp
    /// indicating the date and time at which the origin server believes the
    /// selected representation was last modified, as determined at the
    /// conclusion of handling the request.
    ///
    /// # ABNF
    /// ```plain
    /// Expires = HTTP-date
    /// ```
    ///
    /// # Example Values
    /// * `Sat, 29 Oct 1994 19:43:31 GMT`
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::{SystemTime, Duration};
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::LastModified;
    ///
    /// let mut builder = HttpResponse::Ok();
    /// let modified = SystemTime::now() - Duration::from_secs(60 * 60 * 24);
    /// builder.insert_header(
    ///     LastModified(modified.into())
    /// );
    /// ```
    (LastModified, LAST_MODIFIED) => [HttpDate]

    test_parse_and_format {
        // Test case from RFC
        crate::http::header::common_header_test!(test1, [b"Sat, 29 Oct 1994 19:43:31 GMT"]);
    }
}
