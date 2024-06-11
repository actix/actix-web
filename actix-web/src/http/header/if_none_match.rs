use super::{EntityTag, IF_NONE_MATCH};

crate::http::header::common_header! {
    /// `If-None-Match` header, defined
    /// in [RFC 7232 §3.2](https://datatracker.ietf.org/doc/html/rfc7232#section-3.2)
    ///
    /// The `If-None-Match` header field makes the request method conditional
    /// on a recipient cache or origin server either not having any current
    /// representation of the target resource, when the field-value is "*",
    /// or having a selected representation with an entity-tag that does not
    /// match any of those listed in the field-value.
    ///
    /// A recipient MUST use the weak comparison function when comparing
    /// entity-tags for If-None-Match (Section 2.3.2), since weak entity-tags
    /// can be used for cache validation even if there have been changes to
    /// the representation data.
    ///
    /// # ABNF
    /// ```plain
    /// If-None-Match = "*" / 1#entity-tag
    /// ```
    ///
    /// # Example Values
    /// * `"xyzzy"`
    /// * `W/"xyzzy"`
    /// * `"xyzzy", "r2d2xxxx", "c3piozzzz"`
    /// * `W/"xyzzy", W/"r2d2xxxx", W/"c3piozzzz"`
    /// * `*`
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::IfNoneMatch;
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(IfNoneMatch::Any);
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{IfNoneMatch, EntityTag};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     IfNoneMatch::Items(vec![
    ///         EntityTag::new(false, "xyzzy".to_owned()),
    ///         EntityTag::new(false, "foobar".to_owned()),
    ///         EntityTag::new(false, "bazquux".to_owned()),
    ///     ])
    /// );
    /// ```
    (IfNoneMatch, IF_NONE_MATCH) => {Any / (EntityTag)+}

    test_parse_and_format {
        crate::http::header::common_header_test!(test1, [b"\"xyzzy\""]);
        crate::http::header::common_header_test!(test2, [b"W/\"xyzzy\""]);
        crate::http::header::common_header_test!(test3, [b"\"xyzzy\", \"r2d2xxxx\", \"c3piozzzz\""]);
        crate::http::header::common_header_test!(test4, [b"W/\"xyzzy\", W/\"r2d2xxxx\", W/\"c3piozzzz\""]);
        crate::http::header::common_header_test!(test5, [b"*"]);
    }
}

#[cfg(test)]
mod tests {
    use actix_http::test::TestRequest;

    use super::IfNoneMatch;
    use crate::http::header::{EntityTag, Header, IF_NONE_MATCH};

    #[test]
    fn test_if_none_match() {
        let req = TestRequest::default()
            .insert_header((IF_NONE_MATCH, "*"))
            .finish();

        let mut if_none_match = IfNoneMatch::parse(&req);
        assert_eq!(if_none_match.ok(), Some(IfNoneMatch::Any));

        let req = TestRequest::default()
            .insert_header((IF_NONE_MATCH, &b"\"foobar\", W/\"weak-etag\""[..]))
            .finish();

        if_none_match = Header::parse(&req);
        let mut entities: Vec<EntityTag> = Vec::new();
        let foobar_etag = EntityTag::new_strong("foobar".to_owned());
        let weak_etag = EntityTag::new_weak("weak-etag".to_owned());
        entities.push(foobar_etag);
        entities.push(weak_etag);
        assert_eq!(if_none_match.ok(), Some(IfNoneMatch::Items(entities)));
    }
}
