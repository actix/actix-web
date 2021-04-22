use super::{HttpDate, IF_UNMODIFIED_SINCE};

crate::__define_common_header! {
    /// `If-Unmodified-Since` header, defined in
    /// [RFC7232](http://tools.ietf.org/html/rfc7232#section-3.4)
    ///
    /// The `If-Unmodified-Since` header field makes the request method
    /// conditional on the selected representation's last modification date
    /// being earlier than or equal to the date provided in the field-value.
    /// This field accomplishes the same purpose as If-Match for cases where
    /// the user agent does not have an entity-tag for the representation.
    ///
    /// # ABNF
    ///
    /// ```text
    /// If-Unmodified-Since = HTTP-date
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
    /// use actix_web::http::header::IfUnmodifiedSince;
    ///
    /// let mut builder = HttpResponse::Ok();
    /// let modified = SystemTime::now() - Duration::from_secs(60 * 60 * 24);
    /// builder.insert_header(
    ///     IfUnmodifiedSince(modified.into())
    /// );
    /// ```
    (IfUnmodifiedSince, IF_UNMODIFIED_SINCE) => [HttpDate]

    test_if_unmodified_since {
        // Test case from RFC
        crate::__common_header_test!(test1, vec![b"Sat, 29 Oct 1994 19:43:31 GMT"]);
    }
}
