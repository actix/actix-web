use super::{common_header, EntityTag, IF_MATCH};

common_header! {
    /// `If-Match` header, defined
    /// in [RFC 7232 §3.1](https://datatracker.ietf.org/doc/html/rfc7232#section-3.1)
    ///
    /// The `If-Match` header field makes the request method conditional on
    /// the recipient origin server either having at least one current
    /// representation of the target resource, when the field-value is "*",
    /// or having a current representation of the target resource that has an
    /// entity-tag matching a member of the list of entity-tags provided in
    /// the field-value.
    ///
    /// An origin server MUST use the strong comparison function when
    /// comparing entity-tags for `If-Match`, since the client
    /// intends this precondition to prevent the method from being applied if
    /// there have been any changes to the representation data.
    ///
    /// # ABNF
    /// ```plain
    /// If-Match = "*" / 1#entity-tag
    /// ```
    ///
    /// # Example Values
    /// * `"xyzzy"`
    /// * "xyzzy", "r2d2xxxx", "c3piozzzz"
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::IfMatch;
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(IfMatch::Any);
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{IfMatch, EntityTag};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     IfMatch::Items(vec![
    ///         EntityTag::new(false, "xyzzy".to_owned()),
    ///         EntityTag::new(false, "foobar".to_owned()),
    ///         EntityTag::new(false, "bazquux".to_owned()),
    ///     ])
    /// );
    /// ```
    (IfMatch, IF_MATCH) => {Any / (EntityTag)+}

    test_parse_and_format {
        crate::http::header::common_header_test!(
            test1,
            [b"\"xyzzy\""],
            Some(HeaderField::Items(
                vec![EntityTag::new_strong("xyzzy".to_owned())])));

        crate::http::header::common_header_test!(
            test2,
            [b"\"xyzzy\", \"r2d2xxxx\", \"c3piozzzz\""],
            Some(HeaderField::Items(
                vec![EntityTag::new_strong("xyzzy".to_owned()),
                     EntityTag::new_strong("r2d2xxxx".to_owned()),
                     EntityTag::new_strong("c3piozzzz".to_owned())])));
        crate::http::header::common_header_test!(test3, [b"*"], Some(IfMatch::Any));
    }
}
