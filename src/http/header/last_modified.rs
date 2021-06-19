use super::{HttpDate, LAST_MODIFIED};

crate::__define_common_header! {
    /// `Last-Modified` header, defined in
    /// [RFC7232](http://tools.ietf.org/html/rfc7232#section-2.2)
    ///
    /// The `Last-Modified` header field in a response provides a timestamp
    /// indicating the date and time at which the origin server believes the
    /// selected representation was last modified, as determined at the
    /// conclusion of handling the request.
    ///
    /// # ABNF
    ///
    /// ```text
    /// Expires = HTTP-date
    /// ```
    ///
    /// # Example values
    ///
    /// * `Sat, 29 Oct 1994 19:43:31 GMT`
    ///
    /// # Example
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

        test_last_modified {
            // Test case from RFC
            crate::__common_header_test!(test1, vec![b"Sat, 29 Oct 1994 19:43:31 GMT"]);
        }
}
