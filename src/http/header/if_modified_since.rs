use super::{HttpDate, IF_MODIFIED_SINCE};

crate::__define_common_header! {
    /// `If-Modified-Since` header, defined in
    /// [RFC7232](http://tools.ietf.org/html/rfc7232#section-3.3)
    ///
    /// The `If-Modified-Since` header field makes a GET or HEAD request
    /// method conditional on the selected representation's modification date
    /// being more recent than the date provided in the field-value.
    /// Transfer of the selected representation's data is avoided if that
    /// data has not changed.
    ///
    /// # ABNF
    ///
    /// ```text
    /// If-Unmodified-Since = HTTP-date
    /// ```
    ///
    /// # Example values
    /// * `Sat, 29 Oct 1994 19:43:31 GMT`
    ///
    /// # Example
    ///
    /// ```
    /// use std::time::{SystemTime, Duration};
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::IfModifiedSince;
    ///
    /// let mut builder = HttpResponse::Ok();
    /// let modified = SystemTime::now() - Duration::from_secs(60 * 60 * 24);
    /// builder.insert_header(
    ///     IfModifiedSince(modified.into())
    /// );
    /// ```
    (IfModifiedSince, IF_MODIFIED_SINCE) => [HttpDate]

    test_if_modified_since {
        // Test case from RFC
        crate::__common_header_test!(test1, vec![b"Sat, 29 Oct 1994 19:43:31 GMT"]);
    }
}
