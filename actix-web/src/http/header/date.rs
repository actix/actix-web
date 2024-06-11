use std::time::SystemTime;

use super::{HttpDate, DATE};

crate::http::header::common_header! {
    /// `Date` header, defined
    /// in [RFC 7231 §7.1.1.2](https://datatracker.ietf.org/doc/html/rfc7231#section-7.1.1.2)
    ///
    /// The `Date` header field represents the date and time at which the
    /// message was originated.
    ///
    /// # ABNF
    /// ```plain
    /// Date = HTTP-date
    /// ```
    ///
    /// # Example Values
    /// * `Tue, 15 Nov 1994 08:12:31 GMT`
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::SystemTime;
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::Date;
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     Date(SystemTime::now().into())
    /// );
    /// ```
    (Date, DATE) => [HttpDate]

    test_parse_and_format {
        crate::http::header::common_header_test!(test1, [b"Tue, 15 Nov 1994 08:12:31 GMT"]);
    }
}

impl Date {
    /// Create a date instance set to the current system time
    pub fn now() -> Date {
        Date(SystemTime::now().into())
    }
}
