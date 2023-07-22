use super::common_header;
use crate::http::header;

common_header! {
    /// `Content-Length` header, defined in [RFC 7230 §3.3.2].
    ///
    /// The Content-Length
    ///
    /// # ABNF
    ///
    /// ```plain
    /// Content-Length = 1*DIGIT
    /// ```
    ///
    /// # Example Values
    ///
    /// - `0`
    /// - `3495`
    ///
    /// # Examples
    ///
    /// ```
    /// use actix_web::{http::header::ContentLength, HttpResponse};
    ///
    /// let res_empty = HttpResponse::Ok()
    ///     .insert_header(ContentLength(0));
    ///
    /// let res_fake_cl = HttpResponse::Ok()
    ///     .insert_header(ContentLength(3_495));
    /// ```
    ///
    /// [RFC 7230 §3.3.2]: https://datatracker.ietf.org/doc/html/rfc7230#section-3.3.2
    (ContentLength, header::CONTENT_LENGTH) => [usize]

    test_parse_and_format {
        common_header_test!(no_header, [b""; 0], None);
        common_header_test!(empty_header, [b""; 1], None);

        common_header_test!(zero, [b"0"], Some(ContentLength(0)));
        common_header_test!(one, [b"1"], Some(ContentLength(1)));
        common_header_test!(one_two_three, [b"123"], Some(ContentLength(123)));
        common_header_test!(
            thirty_two_power_plus_one,
            [b"4294967297"],
            Some(ContentLength(4_294_967_297))
        );
        common_header_test!(
            sixty_four_power_minus_one,
            [b"18446744073709551615"],
            Some(ContentLength(18_446_744_073_709_551_615))
        );

        common_header_test!(invalid1, [b"123,567"], None);
        common_header_test!(invalid2, [b"123_567"], None);
    }
}
