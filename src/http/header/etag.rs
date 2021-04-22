use super::{EntityTag, ETAG};

crate::__define_common_header! {
    /// `ETag` header, defined in [RFC7232](http://tools.ietf.org/html/rfc7232#section-2.3)
    ///
    /// The `ETag` header field in a response provides the current entity-tag
    /// for the selected representation, as determined at the conclusion of
    /// handling the request.  An entity-tag is an opaque validator for
    /// differentiating between multiple representations of the same
    /// resource, regardless of whether those multiple representations are
    /// due to resource state changes over time, content negotiation
    /// resulting in multiple representations being valid at the same time,
    /// or both.  An entity-tag consists of an opaque quoted string, possibly
    /// prefixed by a weakness indicator.
    ///
    /// # ABNF
    ///
    /// ```text
    /// ETag       = entity-tag
    /// ```
    ///
    /// # Example values
    ///
    /// * `"xyzzy"`
    /// * `W/"xyzzy"`
    /// * `""`
    ///
    /// # Examples
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{ETag, EntityTag};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     ETag(EntityTag::new(false, "xyzzy".to_owned()))
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{ETag, EntityTag};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     ETag(EntityTag::new(true, "xyzzy".to_owned()))
    /// );
    /// ```
    (ETag, ETAG) => [EntityTag]

    test_etag {
        // From the RFC
        crate::__common_header_test!(test1,
            vec![b"\"xyzzy\""],
            Some(ETag(EntityTag::new(false, "xyzzy".to_owned()))));
        crate::__common_header_test!(test2,
            vec![b"W/\"xyzzy\""],
            Some(ETag(EntityTag::new(true, "xyzzy".to_owned()))));
        crate::__common_header_test!(test3,
            vec![b"\"\""],
            Some(ETag(EntityTag::new(false, "".to_owned()))));
        // Own tests
        crate::__common_header_test!(test4,
            vec![b"\"foobar\""],
            Some(ETag(EntityTag::new(false, "foobar".to_owned()))));
        crate::__common_header_test!(test5,
            vec![b"\"\""],
            Some(ETag(EntityTag::new(false, "".to_owned()))));
        crate::__common_header_test!(test6,
            vec![b"W/\"weak-etag\""],
            Some(ETag(EntityTag::new(true, "weak-etag".to_owned()))));
        crate::__common_header_test!(test7,
            vec![b"W/\"\x65\x62\""],
            Some(ETag(EntityTag::new(true, "\u{0065}\u{0062}".to_owned()))));
        crate::__common_header_test!(test8,
            vec![b"W/\"\""],
            Some(ETag(EntityTag::new(true, "".to_owned()))));
        crate::__common_header_test!(test9,
            vec![b"no-dquotes"],
            None::<ETag>);
        crate::__common_header_test!(test10,
            vec![b"w/\"the-first-w-is-case-sensitive\""],
            None::<ETag>);
        crate::__common_header_test!(test11,
            vec![b""],
            None::<ETag>);
        crate::__common_header_test!(test12,
            vec![b"\"unmatched-dquotes1"],
            None::<ETag>);
        crate::__common_header_test!(test13,
            vec![b"unmatched-dquotes2\""],
            None::<ETag>);
        crate::__common_header_test!(test14,
            vec![b"matched-\"dquotes\""],
            None::<ETag>);
        crate::__common_header_test!(test15,
            vec![b"\""],
            None::<ETag>);
    }
}
