use http::Method;
use header::http;

header! {
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
    /// ```rust
    /// # extern crate http;
    /// # extern crate actix_web;
    /// use actix_web::httpcodes::HttpOk;
    /// use actix_web::header::Allow;
    /// use http::Method;
    ///
    /// # fn main() {
    /// let mut builder = HttpOk.build();
    /// builder.set(
    ///     Allow(vec![Method::GET])
    /// );
    /// # }
    /// ```
    ///
    /// ```rust
    /// # extern crate http;
    /// # extern crate actix_web;
    /// use actix_web::httpcodes::HttpOk;
    /// use actix_web::header::Allow;
    /// use http::Method;
    ///
    /// # fn main() {
    /// let mut builder = HttpOk.build();
    /// builder.set(
    ///     Allow(vec![
    ///         Method::GET,
    ///         Method::POST,
    ///         Method::PATCH,
    ///     ])
    /// );
    /// # }
    /// ```
    (Allow, http::ALLOW) => (Method)*

    test_allow {
        // From the RFC
        test_header!(
            test1,
            vec![b"GET, HEAD, PUT"],
            Some(HeaderField(vec![Method::GET, Method::HEAD, Method::PUT])));
        // Own tests
        test_header!(
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
        test_header!(
            test3,
            vec![b""],
            Some(HeaderField(Vec::<Method>::new())));
    }
}
