use crate::http::header;
use actix_http::http::Method;

crate::__define_common_header! {
    /// `Allow` header, defined in [RFC7231](http://tools.ietf.org/html/rfc7231#section-7.4.1)
    ///
    /// The `Allow` header field lists the set of methods advertised as
    /// supported by the target resource.  The purpose of this field is
    /// strictly to inform the recipient of valid request methods associated
    /// with the resource.
    ///
    /// # ABNF
    ///
    /// ```text
    /// Allow = #method
    /// ```
    ///
    /// # Example values
    /// * `GET, HEAD, PUT`
    /// * `OPTIONS, GET, PUT, POST, DELETE, HEAD, TRACE, CONNECT, PATCH, fOObAr`
    /// * ``
    ///
    /// # Examples
    ///
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

    test_allow {
        // From the RFC
        crate::__common_header_test!(
            test1,
            vec![b"GET, HEAD, PUT"],
            Some(HeaderField(vec![Method::GET, Method::HEAD, Method::PUT])));
        // Own tests
        crate::__common_header_test!(
            test2,
            vec![b"OPTIONS, GET, PUT, POST, DELETE, HEAD, TRACE, CONNECT, PATCH"],
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
        crate::__common_header_test!(
            test3,
            vec![b""],
            Some(HeaderField(Vec::<Method>::new())));
    }
}
