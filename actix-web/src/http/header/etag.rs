use super::{EntityTag, ETAG};

crate::http::header::common_header! {
    /// `ETag` header, defined in
    /// [RFC 7232 §2.3](https://datatracker.ietf.org/doc/html/rfc7232#section-2.3)
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
    /// ```plain
    /// ETag       = entity-tag
    /// ```
    ///
    /// # Example Values
    /// * `"xyzzy"`
    /// * `W/"xyzzy"`
    /// * `""`
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{ETag, EntityTag};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     ETag(EntityTag::new_strong("xyzzy".to_owned()))
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{ETag, EntityTag};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     ETag(EntityTag::new_weak("xyzzy".to_owned()))
    /// );
    /// ```
    (ETag, ETAG) => [EntityTag]

    test_parse_and_format {
        // From the RFC
        crate::http::header::common_header_test!(test1,
            [b"\"xyzzy\""],
            Some(ETag(EntityTag::new_strong("xyzzy".to_owned()))));
        crate::http::header::common_header_test!(test2,
            [b"W/\"xyzzy\""],
            Some(ETag(EntityTag::new_weak("xyzzy".to_owned()))));
        crate::http::header::common_header_test!(test3,
            [b"\"\""],
            Some(ETag(EntityTag::new_strong("".to_owned()))));
        // Own tests
        crate::http::header::common_header_test!(test4,
            [b"\"foobar\""],
            Some(ETag(EntityTag::new_strong("foobar".to_owned()))));
        crate::http::header::common_header_test!(test5,
            [b"\"\""],
            Some(ETag(EntityTag::new_strong("".to_owned()))));
        crate::http::header::common_header_test!(test6,
            [b"W/\"weak-etag\""],
            Some(ETag(EntityTag::new_weak("weak-etag".to_owned()))));
        crate::http::header::common_header_test!(test7,
            [b"W/\"\x65\x62\""],
            Some(ETag(EntityTag::new_weak("\u{0065}\u{0062}".to_owned()))));
        crate::http::header::common_header_test!(test8,
            [b"W/\"\""],
            Some(ETag(EntityTag::new_weak("".to_owned()))));
        crate::http::header::common_header_test!(test9,
            [b"no-dquotes"],
            None::<ETag>);
        crate::http::header::common_header_test!(test10,
            [b"w/\"the-first-w-is-case-sensitive\""],
            None::<ETag>);
        crate::http::header::common_header_test!(test11,
            [b""],
            None::<ETag>);
        crate::http::header::common_header_test!(test12,
            [b"\"unmatched-dquotes1"],
            None::<ETag>);
        crate::http::header::common_header_test!(test13,
            [b"unmatched-dquotes2\""],
            None::<ETag>);
        crate::http::header::common_header_test!(test14,
            [b"matched-\"dquotes\""],
            None::<ETag>);
        crate::http::header::common_header_test!(test15,
            [b"\""],
            None::<ETag>);
    }
}
