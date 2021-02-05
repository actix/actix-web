use crate::header::{EntityTag, IF_MATCH};

header! {
    /// `If-Match` header, defined in
    /// [RFC7232](https://tools.ietf.org/html/rfc7232#section-3.1)
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
    ///
    /// ```text
    /// If-Match = "*" / 1#entity-tag
    /// ```
    ///
    /// # Example values
    ///
    /// * `"xyzzy"`
    /// * "xyzzy", "r2d2xxxx", "c3piozzzz"
    ///
    /// # Examples
    ///
    /// ```
    /// use actix_http::Response;
    /// use actix_http::http::header::IfMatch;
    ///
    /// let mut builder = Response::Ok();
    /// builder.insert_header(IfMatch::Any);
    /// ```
    ///
    /// ```
    /// use actix_http::Response;
    /// use actix_http::http::header::{IfMatch, EntityTag};
    ///
    /// let mut builder = Response::Ok();
    /// builder.insert_header(
    ///     IfMatch::Items(vec![
    ///         EntityTag::new(false, "xyzzy".to_owned()),
    ///         EntityTag::new(false, "foobar".to_owned()),
    ///         EntityTag::new(false, "bazquux".to_owned()),
    ///     ])
    /// );
    /// ```
    (IfMatch, IF_MATCH) => {Any / (EntityTag)+}

    test_if_match {
        test_header!(
            test1,
            vec![b"\"xyzzy\""],
            Some(HeaderField::Items(
                vec![EntityTag::new(false, "xyzzy".to_owned())])));
        test_header!(
            test2,
            vec![b"\"xyzzy\", \"r2d2xxxx\", \"c3piozzzz\""],
            Some(HeaderField::Items(
                vec![EntityTag::new(false, "xyzzy".to_owned()),
                     EntityTag::new(false, "r2d2xxxx".to_owned()),
                     EntityTag::new(false, "c3piozzzz".to_owned())])));
        test_header!(test3, vec![b"*"], Some(IfMatch::Any));
    }
}
