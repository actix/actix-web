use actix_http::header::QualityItem;

use super::{common_header, Encoding, Preference, Quality};
use crate::http::header;

common_header! {
    /// `Accept-Encoding` header, defined
    /// in [RFC 7231](https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.4)
    ///
    /// The `Accept-Encoding` header field can be used by user agents to
    /// indicate what response content-codings are
    /// acceptable in the response.  An  `identity` token is used as a synonym
    /// for "no encoding" in order to communicate when no encoding is
    /// preferred.
    ///
    /// # ABNF
    /// ```plain
    /// Accept-Encoding  = #( codings [ weight ] )
    /// codings          = content-coding / "identity" / "*"
    /// ```
    ///
    /// # Example Values
    /// * `compress, gzip`
    /// * ``
    /// * `*`
    /// * `compress;q=0.5, gzip;q=1`
    /// * `gzip;q=1.0, identity; q=0.5, *;q=0`
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptEncoding, Encoding, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptEncoding(vec![QualityItem::max(Encoding::Chunked)])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptEncoding, Encoding, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptEncoding(vec![
    ///         QualityItem::max(Encoding::Chunked),
    ///         QualityItem::max(Encoding::Gzip),
    ///         QualityItem::max(Encoding::Deflate),
    ///     ])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptEncoding, Encoding, QualityItem, q};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptEncoding(vec![
    ///         QualityItem::max(Encoding::Chunked),
    ///         QualityItem::new(Encoding::Gzip, q(0.60)),
    ///         QualityItem::min(Encoding::EncodingExt("*".to_owned())),
    ///     ])
    /// );
    /// ```
    (AcceptEncoding, header::ACCEPT_ENCODING) => (QualityItem<Preference<Encoding>>)*

    test_parse_and_format {
        common_header_test!(no_headers, vec![b""; 0], Some(AcceptEncoding(vec![])));
        common_header_test!(empty_header, vec![b""; 1], Some(AcceptEncoding(vec![])));

        // From the RFC
        common_header_test!(
            order_of_appearance,
            vec![b"compress, gzip"],
            Some(AcceptEncoding(vec![
                QualityItem::max(Preference::Specific(Encoding::Compress)),
                QualityItem::max(Preference::Specific(Encoding::Gzip)),
            ]))
        );

        common_header_test!(any, vec![b"*"], Some(AcceptEncoding(vec![
            QualityItem::max(Preference::Any),
        ])));

        // Note: Removed quality 1 from gzip
        common_header_test!(implicit_quality, vec![b"gzip, identity; q=0.5, *;q=0"]);

        // Note: Removed quality 1 from gzip
        common_header_test!(implicit_quality_out_of_order, vec![b"compress;q=0.5, gzip"]);

        common_header_test!(
            only_gzip_no_identity,
            vec![b"gzip, *; q=0"],
            Some(AcceptEncoding(vec![
                QualityItem::max(Preference::Specific(Encoding::Gzip)),
                QualityItem::min(Preference::Any),
            ]))
        );
    }
}

impl AcceptEncoding {
    // TODO: method for getting best content encoding based on q-factors, available from server side
    // and if none are acceptable return None

    /// Extracts the most preferable encoding, accounting for [q-factor weighting].
    ///
    /// If no q-factors are provided, the first encoding is chosen. Note that items without
    /// q-factors are given the maximum preference value.
    ///
    /// As per the spec, returns [`Preference::Any`] if contained list is empty.
    ///
    /// [q-factor weighting]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.2
    pub fn preference(&self) -> Preference<Encoding> {
        let mut max_item = None;
        let mut max_pref = Quality::MIN;

        // uses manual max lookup loop since we want the first occurrence in the case of same
        // preference but `Iterator::max_by_key` would give us the last occurrence

        for pref in &self.0 {
            // only change if strictly greater
            // equal items, even while unsorted, still have higher preference if they appear first
            if pref.quality > max_pref {
                max_pref = pref.quality;
                max_item = Some(pref.item.clone());
            }
        }

        max_item.unwrap_or(Preference::Any)
    }

    /// Returns a sorted list of encodings from highest to lowest precedence, accounting
    /// for [q-factor weighting].
    ///
    /// [q-factor weighting]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.2
    pub fn ranked(&self) -> Vec<Preference<Encoding>> {
        if self.0.is_empty() {
            return vec![];
        }

        let mut types = self.0.clone();

        // use stable sort so items with equal q-factor retain listed order
        types.sort_by(|a, b| {
            // sort by q-factor descending
            b.quality.cmp(&a.quality)
        });

        types.into_iter().map(|qitem| qitem.item).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::header::*;

    #[test]
    fn ranking_precedence() {
        let test = AcceptLanguage(vec![]);
        assert!(test.ranked().is_empty());

        let test = AcceptLanguage(vec![QualityItem::max("gzip".parse().unwrap())]);
        assert_eq!(test.ranked(), vec!["gzip".parse().unwrap()]);

        let test = AcceptLanguage(vec![
            QualityItem::new("gzip".parse().unwrap(), q(0.900)),
            QualityItem::new("*".parse().unwrap(), q(0.700)),
            QualityItem::new("br".parse().unwrap(), q(1.0)),
        ]);
        assert_eq!(
            test.ranked(),
            vec![
                "br".parse().unwrap(),
                "gzip".parse().unwrap(),
                "*".parse().unwrap(),
            ]
        );

        let test = AcceptLanguage(vec![
            QualityItem::max("br".parse().unwrap()),
            QualityItem::max("gzip".parse().unwrap()),
            QualityItem::max("*".parse().unwrap()),
        ]);
        assert_eq!(
            test.ranked(),
            vec![
                "br".parse().unwrap(),
                "gzip".parse().unwrap(),
                "*".parse().unwrap(),
            ]
        );
    }

    #[test]
    fn preference_selection() {
        assert_eq!(AcceptLanguage(vec![]).preference(), Preference::Any);

        assert_eq!(
            AcceptLanguage(vec!["compress;q=0; *;q=0".parse().unwrap()]).preference(),
            Preference::Any
        );

        assert_eq!(
            AcceptLanguage(vec!["identity;q=0; *;q=0".parse().unwrap()]).preference(),
            Preference::Any
        );

        let test = AcceptLanguage(vec![
            QualityItem::new("br".parse().unwrap(), q(0.900)),
            QualityItem::new("gzip".parse().unwrap(), q(1.0)),
            QualityItem::new("*".parse().unwrap(), q(0.500)),
        ]);
        assert_eq!(
            test.preference(),
            Preference::Specific("gzip".parse().unwrap())
        );

        let test = AcceptLanguage(vec![
            QualityItem::max("br".parse().unwrap()),
            QualityItem::max("gzip".parse().unwrap()),
            QualityItem::max("*".parse().unwrap()),
        ]);
        assert_eq!(
            test.preference(),
            Preference::Specific("br".parse().unwrap())
        );
    }
}
