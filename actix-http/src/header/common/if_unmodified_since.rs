use crate::header::{HttpDate, IF_UNMODIFIED_SINCE};

header! {
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
    /// use actix_http::Response;
    /// use actix_http::http::header::IfUnmodifiedSince;
    ///
    /// let mut builder = Response::Ok();
    /// let modified = SystemTime::now() - Duration::from_secs(60 * 60 * 24);
    /// builder.insert_header(
    ///     IfUnmodifiedSince(modified.into())
    /// );
    /// ```
    (IfUnmodifiedSince, IF_UNMODIFIED_SINCE) => [HttpDate]

    test_if_unmodified_since {
        // Test case from RFC
        test_header!(test1, vec![b"Sat, 29 Oct 1994 19:43:31 GMT"]);
    }
}
