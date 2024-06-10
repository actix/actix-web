use actix_http::Method;

use crate::http::header;

crate::http::header::common_header! {
    /// `Allow` header, defined
    /// in [RFC 7231 §7.4.1](https://datatracker.ietf.org/doc/html/rfc7231#section-7.4.1)
    ///
    /// The `Allow` header field lists the set of methods advertised as
    /// supported by the target resource. The purpose of this field is
    /// strictly to inform the recipient of valid request methods associated
    /// with the resource.
    ///
    /// # ABNF
    /// ```plain
    /// Allow = #method
    /// ```
    ///
    /// # Example Values
    /// * `GET, HEAD, PUT`
    /// * `OPTIONS, GET, PUT, POST, DELETE, HEAD, TRACE, CONNECT, PATCH, fOObAr`
    /// * ``
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::{header::Allow, Method};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     Allow(vec![Method::GET])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::{header::Allow, Method};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     Allow(vec![
    ///         Method::GET,
    ///         Method::POST,
    ///         Method::PATCH,
    ///     ])
    /// );
    /// ```
    (Allow, header::ALLOW) => (Method)*

    test_parse_and_format {
        // from the RFC

        crate::http::header::common_header_test!(
            test1,
            [b"GET, HEAD, PUT"],
            Some(HeaderField(vec![Method::GET, Method::HEAD, Method::PUT])));

        // other tests

        crate::http::header::common_header_test!(
            test2,
            [b"OPTIONS, GET, PUT, POST, DELETE, HEAD, TRACE, CONNECT, PATCH"],
            Some(HeaderField(vec![
                Method::OPTIONS,
                Method::GET,
                Method::PUT,
                Method::POST,
                Method::DELETE,
                Method::HEAD,
                Method::TRACE,
                Method::CONNECT,
                Method::PATCH])));

        crate::http::header::common_header_test!(
            test3,
            [b""],
            Some(HeaderField(Vec::<Method>::new())));
    }
}
