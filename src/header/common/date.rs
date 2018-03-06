use std::time::SystemTime;
use header::{http, HttpDate};


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
    /// use actix_web::httpcodes;
    /// use actix_web::header::Date;
    /// use std::time::SystemTime;
    ///
    /// let mut builder = httpcodes::HttpOk.build();
    /// builder.set(Date(SystemTime::now().into()));
    /// ```
    (Date, http::DATE) => [HttpDate]

    test_date {
        test_header!(test1, vec![b"Tue, 15 Nov 1994 08:12:31 GMT"]);
    }
}

impl Date {
    pub fn now() -> Date {
        Date(SystemTime::now().into())
    }
}
