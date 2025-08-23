use super::{HttpDate, EXPIRES};

crate::http::header::common_header! {
    /// `Expires` header, defined
    /// in [RFC 7234 §5.3](https://datatracker.ietf.org/doc/html/rfc7234#section-5.3)
    ///
    /// The `Expires` header field gives the date/time after which the
    /// response is considered stale.
    ///
    /// The presence of an Expires field does not imply that the original
    /// resource will change or cease to exist at, before, or after that
    /// time.
    ///
    /// # ABNF
    /// ```plain
    /// Expires = HTTP-date
    /// ```
    ///
    /// # Example Values
    /// * `Thu, 01 Dec 1994 16:00:00 GMT`
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::{SystemTime, Duration};
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::Expires;
    ///
    /// let mut builder = HttpResponse::Ok();
    /// let expiration = SystemTime::now() + Duration::from_secs(60 * 60 * 24);
    /// builder.insert_header(
    ///     Expires(expiration.into())
    /// );
    /// ```
    (Expires, EXPIRES) => [HttpDate]

    test_parse_and_format {
        // Test case from RFC
        crate::http::header::common_header_test!(test1, [b"Thu, 01 Dec 1994 16:00:00 GMT"]);
    }
}
