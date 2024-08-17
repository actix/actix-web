use super::{Uri, LOCATION};

crate::http::header::common_header! {
    /// `Location` header, defined
    /// in [RFC 7231 §7.1.2](https://datatracker.ietf.org/doc/html/rfc7231#section-7.1.2)
    ///
    /// The "Location" header field is used in some responses to refer to a
    /// specific resource in relation to the response.  The type of
    /// relationship is defined by the combination of request method and
    /// status code semantics.
    ///
    /// # ABNF
    /// ```plain
    /// Location = URI-reference
    /// ```
    ///
    /// # Example Values
    /// * `http://www.example.org/hypertext/Overview.html`
    ///
    /// # Examples
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_http::Uri;
    /// use actix_web::http::header::Location;
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     Location("http://www.example.org".parse::<Uri>().unwrap())
    /// );
    /// ```
    (Location, LOCATION) => [Uri]

    test_parse_and_format {
        crate::http::header::common_header_test!(test1, [b"http://www.example.org/hypertext/Overview.html"]);
    }
}
