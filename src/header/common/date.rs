use crate::header::{HttpDate, DATE};
use std::time::SystemTime;

header! {
    /// `Date` header, defined in [RFC7231](http://tools.ietf.org/html/rfc7231#section-7.1.1.2)
    ///
    /// The `Date` header field represents the date and time at which the
    /// message was originated.
    ///
    /// # ABNF
    ///
    /// ```text
    /// Date = HTTP-date
    /// ```
    ///
    /// # Example values
    ///
    /// * `Tue, 15 Nov 1994 08:12:31 GMT`
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::Response;
    /// use actix_http::http::header::Date;
    /// use std::time::SystemTime;
    ///
    /// let mut builder = Response::Ok();
    /// builder.set(Date(SystemTime::now().into()));
    /// ```
    (Date, DATE) => [HttpDate]

    test_date {
        test_header!(test1, vec![b"Tue, 15 Nov 1994 08:12:31 GMT"]);
    }
}

impl Date {
    /// Create a date instance set to the current system time
    pub fn now() -> Date {
        Date(SystemTime::now().into())
    }
}
