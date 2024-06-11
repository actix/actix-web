use std::collections::HashSet;

use super::{common_header, ContentEncoding, Encoding, Preference, Quality, QualityItem};
use crate::http::header;

common_header! {
    /// `Accept-Encoding` header, defined
    /// in [RFC 7231](https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.4)
    ///
    /// The `Accept-Encoding` header field can be used by user agents to indicate what response
    /// content-codings are acceptable in the response. An `identity` token is used as a synonym
    /// for "no encoding" in order to communicate when no encoding is preferred.
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
    /// use actix_web::http::header::{AcceptEncoding, Encoding, Preference, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptEncoding(vec![QualityItem::max(Preference::Specific(Encoding::gzip()))])
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
    ///         "gzip".parse().unwrap(),
    ///         "br".parse().unwrap(),
    ///     ])
    /// );
    /// ```
    (AcceptEncoding, header::ACCEPT_ENCODING) => (QualityItem<Preference<Encoding>>)*

    test_parse_and_format {
        common_header_test!(no_headers, [b""; 0], Some(AcceptEncoding(vec![])));
        common_header_test!(empty_header, [b""; 1], Some(AcceptEncoding(vec![])));

        common_header_test!(
            order_of_appearance,
            [b"br, gzip"],
            Some(AcceptEncoding(vec![
                QualityItem::max(Preference::Specific(Encoding::brotli())),
                QualityItem::max(Preference::Specific(Encoding::gzip())),
            ]))
        );

        common_header_test!(any, [b"*"], Some(AcceptEncoding(vec![
            QualityItem::max(Preference::Any),
        ])));

        // Note: Removed quality 1 from gzip
        common_header_test!(implicit_quality, [b"gzip, identity; q=0.5, *;q=0"]);

        // Note: Removed quality 1 from gzip
        common_header_test!(implicit_quality_out_of_order, [b"compress;q=0.5, gzip"]);

        common_header_test!(
            only_gzip_no_identity,
            [b"gzip, *; q=0"],
            Some(AcceptEncoding(vec![
                QualityItem::max(Preference::Specific(Encoding::gzip())),
                QualityItem::zero(Preference::Any),
            ]))
        );
    }
}

impl AcceptEncoding {
    /// Selects the most acceptable encoding according to client preference and supported types.
    ///
    /// The "identity" encoding is not assumed and should be included in the `supported` iterator
    /// if a non-encoded representation can be selected.
    ///
    /// If `None` is returned, this indicates that none of the supported encodings are acceptable to
    /// the client. The caller should generate a 406 Not Acceptable response (unencoded) that
    /// includes the server's supported encodings in the body plus a [`Vary`] header.
    ///
    /// [`Vary`]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Vary
    pub fn negotiate<'a>(&self, supported: impl Iterator<Item = &'a Encoding>) -> Option<Encoding> {
        // 1. If no Accept-Encoding field is in the request, any content-coding is considered
        // acceptable by the user agent.

        let supported_set = supported.collect::<HashSet<_>>();

        if supported_set.is_empty() {
            return None;
        }

        if self.0.is_empty() {
            // though it is not recommended to encode in this case, return identity encoding
            return Some(Encoding::identity());
        }

        // 2. If the representation has no content-coding, then it is acceptable by default unless
        // specifically excluded by the Accept-Encoding field stating either "identity;q=0" or
        // "*;q=0" without a more specific entry for "identity".

        let acceptable_items = self.ranked_items().collect::<Vec<_>>();

        let identity_acceptable = is_identity_acceptable(&acceptable_items);
        let identity_supported = supported_set.contains(&Encoding::identity());

        if identity_acceptable && identity_supported && supported_set.len() == 1 {
            return Some(Encoding::identity());
        }

        // 3. If the representation's content-coding is one of the content-codings listed in the
        // Accept-Encoding field, then it is acceptable unless it is accompanied by a qvalue of 0.

        // 4. If multiple content-codings are acceptable, then the acceptable content-coding with
        // the highest non-zero qvalue is preferred.

        let matched = acceptable_items
            .into_iter()
            .filter(|q| q.quality > Quality::ZERO)
            // search relies on item list being in descending order of quality
            .find(|q| {
                let enc = &q.item;
                matches!(enc, Preference::Specific(enc) if supported_set.contains(enc))
            })
            .map(|q| q.item);

        match matched {
            Some(Preference::Specific(enc)) => Some(enc),

            _ if identity_acceptable => Some(Encoding::identity()),

            _ => None,
        }
    }

    /// Extracts the most preferable encoding, accounting for [q-factor weighting].
    ///
    /// If no q-factors are provided, we prefer brotli > zstd > gzip. Note that items without
    /// q-factors are given the maximum preference value.
    ///
    /// As per the spec, returns [`Preference::Any`] if acceptable list is empty. Though, if this is
    /// returned, it is recommended to use an un-encoded representation.
    ///
    /// If `None` is returned, it means that the client has signalled that no representations
    /// are acceptable. This should never occur for a well behaved user-agent.
    ///
    /// [q-factor weighting]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.2
    pub fn preference(&self) -> Option<Preference<Encoding>> {
        // empty header indicates no preference
        if self.0.is_empty() {
            return Some(Preference::Any);
        }

        let mut max_item = None;
        let mut max_pref = Quality::ZERO;
        let mut max_rank = 0;

        // uses manual max lookup loop since we want the first occurrence in the case of same
        // preference but `Iterator::max_by_key` would give us the last occurrence

        for pref in &self.0 {
            // only change if strictly greater
            // equal items, even while unsorted, still have higher preference if they appear first

            let rank = encoding_rank(pref);

            if (pref.quality, rank) > (max_pref, max_rank) {
                max_pref = pref.quality;
                max_item = Some(pref.item.clone());
                max_rank = rank;
            }
        }

        // Return max_item if any items were above 0 quality...
        max_item.or_else(|| {
            // ...or else check for "*" or "identity". We can elide quality checks since
            // entering this block means all items had "q=0".
            match self.0.iter().find(|pref| {
                matches!(
                    pref.item,
                    Preference::Any
                        | Preference::Specific(Encoding::Known(ContentEncoding::Identity))
                )
            }) {
                // "identity" or "*" found so no representation is acceptable
                Some(_) => None,

                // implicit "identity" is acceptable
                None => Some(Preference::Specific(Encoding::identity())),
            }
        })
    }

    /// Returns a sorted list of encodings from highest to lowest precedence, accounting
    /// for [q-factor weighting].
    ///
    /// If no q-factors are provided, we prefer brotli > zstd > gzip.
    ///
    /// [q-factor weighting]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.2
    pub fn ranked(&self) -> Vec<Preference<Encoding>> {
        self.ranked_items().map(|q| q.item).collect()
    }

    fn ranked_items(&self) -> impl Iterator<Item = QualityItem<Preference<Encoding>>> {
        if self.0.is_empty() {
            return Vec::new().into_iter();
        }

        let mut types = self.0.clone();

        // use stable sort so items with equal q-factor retain listed order
        types.sort_by(|a, b| {
            // sort by q-factor descending then server ranking descending

            b.quality
                .cmp(&a.quality)
                .then(encoding_rank(b).cmp(&encoding_rank(a)))
        });

        types.into_iter()
    }
}

/// Returns server-defined encoding ranking.
fn encoding_rank(qv: &QualityItem<Preference<Encoding>>) -> u8 {
    // ensure that q=0 items are never sorted above identity encoding
    // invariant: sorting methods calling this fn use first-on-equal approach
    if qv.quality == Quality::ZERO {
        return 0;
    }

    match qv.item {
        Preference::Specific(Encoding::Known(ContentEncoding::Brotli)) => 5,
        Preference::Specific(Encoding::Known(ContentEncoding::Zstd)) => 4,
        Preference::Specific(Encoding::Known(ContentEncoding::Gzip)) => 3,
        Preference::Specific(Encoding::Known(ContentEncoding::Deflate)) => 2,
        Preference::Any => 0,
        Preference::Specific(Encoding::Known(ContentEncoding::Identity)) => 0,
        Preference::Specific(Encoding::Known(_)) => 1,
        Preference::Specific(Encoding::Unknown(_)) => 1,
    }
}

/// Returns true if "identity" is an acceptable encoding.
///
/// Internal algorithm relies on item list being in descending order of quality.
fn is_identity_acceptable(items: &'_ [QualityItem<Preference<Encoding>>]) -> bool {
    if items.is_empty() {
        return true;
    }

    // Loop algorithm depends on items being sorted in descending order of quality. As such, it
    // is sufficient to return (q > 0) when reaching either an "identity" or "*" item.
    for q in items {
        match (q.quality, &q.item) {
            // occurrence of "identity;q=n"; return true if quality is non-zero
            (q, Preference::Specific(Encoding::Known(ContentEncoding::Identity))) => {
                return q > Quality::ZERO
            }

            // occurrence of "*;q=n"; return true if quality is non-zero
            (q, Preference::Any) => return q > Quality::ZERO,

            _ => {}
        }
    }

    // implicit acceptable identity
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::header::*;

    macro_rules! accept_encoding {
        () => { AcceptEncoding(vec![]) };
        ($($q:expr),+ $(,)?) => { AcceptEncoding(vec![$($q.parse().unwrap()),+]) };
    }

    /// Parses an encoding string.
    fn enc(enc: &str) -> Preference<Encoding> {
        enc.parse().unwrap()
    }

    #[test]
    fn detect_identity_acceptable() {
        macro_rules! accept_encoding_ranked {
            () => { accept_encoding!().ranked_items().collect::<Vec<_>>() };
            ($($q:expr),+ $(,)?) => { accept_encoding!($($q),+).ranked_items().collect::<Vec<_>>() };
        }

        let test = accept_encoding_ranked!();
        assert!(is_identity_acceptable(&test));
        let test = accept_encoding_ranked!("gzip");
        assert!(is_identity_acceptable(&test));
        let test = accept_encoding_ranked!("gzip", "br");
        assert!(is_identity_acceptable(&test));
        let test = accept_encoding_ranked!("gzip", "*;q=0.1");
        assert!(is_identity_acceptable(&test));
        let test = accept_encoding_ranked!("gzip", "identity;q=0.1");
        assert!(is_identity_acceptable(&test));
        let test = accept_encoding_ranked!("gzip", "identity;q=0.1", "*;q=0");
        assert!(is_identity_acceptable(&test));
        let test = accept_encoding_ranked!("gzip", "*;q=0", "identity;q=0.1");
        assert!(is_identity_acceptable(&test));

        let test = accept_encoding_ranked!("gzip", "*;q=0");
        assert!(!is_identity_acceptable(&test));
        let test = accept_encoding_ranked!("gzip", "identity;q=0");
        assert!(!is_identity_acceptable(&test));
        let test = accept_encoding_ranked!("gzip", "identity;q=0", "*;q=0");
        assert!(!is_identity_acceptable(&test));
        let test = accept_encoding_ranked!("gzip", "*;q=0", "identity;q=0");
        assert!(!is_identity_acceptable(&test));
    }

    #[test]
    fn encoding_negotiation() {
        // no preference
        let test = accept_encoding!();
        assert_eq!(test.negotiate([].iter()), None);

        let test = accept_encoding!();
        assert_eq!(
            test.negotiate([Encoding::identity()].iter()),
            Some(Encoding::identity()),
        );

        let test = accept_encoding!("identity;q=0");
        assert_eq!(test.negotiate([Encoding::identity()].iter()), None);

        let test = accept_encoding!("*;q=0");
        assert_eq!(test.negotiate([Encoding::identity()].iter()), None);

        let test = accept_encoding!();
        assert_eq!(
            test.negotiate([Encoding::gzip(), Encoding::identity()].iter()),
            Some(Encoding::identity()),
        );

        let test = accept_encoding!("gzip");
        assert_eq!(
            test.negotiate([Encoding::gzip(), Encoding::identity()].iter()),
            Some(Encoding::gzip()),
        );
        assert_eq!(
            test.negotiate([Encoding::brotli(), Encoding::identity()].iter()),
            Some(Encoding::identity()),
        );
        assert_eq!(
            test.negotiate([Encoding::brotli(), Encoding::gzip(), Encoding::identity()].iter()),
            Some(Encoding::gzip()),
        );

        let test = accept_encoding!("gzip", "identity;q=0");
        assert_eq!(
            test.negotiate([Encoding::gzip(), Encoding::identity()].iter()),
            Some(Encoding::gzip()),
        );
        assert_eq!(
            test.negotiate([Encoding::brotli(), Encoding::identity()].iter()),
            None
        );

        let test = accept_encoding!("gzip", "*;q=0");
        assert_eq!(
            test.negotiate([Encoding::gzip(), Encoding::identity()].iter()),
            Some(Encoding::gzip()),
        );
        assert_eq!(
            test.negotiate([Encoding::brotli(), Encoding::identity()].iter()),
            None
        );

        let test = accept_encoding!("gzip", "deflate", "br");
        assert_eq!(
            test.negotiate([Encoding::gzip(), Encoding::identity()].iter()),
            Some(Encoding::gzip()),
        );
        assert_eq!(
            test.negotiate([Encoding::brotli(), Encoding::identity()].iter()),
            Some(Encoding::brotli())
        );
        assert_eq!(
            test.negotiate([Encoding::deflate(), Encoding::identity()].iter()),
            Some(Encoding::deflate())
        );
        assert_eq!(
            test.negotiate([Encoding::gzip(), Encoding::deflate(), Encoding::identity()].iter()),
            Some(Encoding::gzip())
        );
        assert_eq!(
            test.negotiate([Encoding::gzip(), Encoding::brotli(), Encoding::identity()].iter()),
            Some(Encoding::brotli())
        );
        assert_eq!(
            test.negotiate([Encoding::brotli(), Encoding::gzip(), Encoding::identity()].iter()),
            Some(Encoding::brotli())
        );
    }

    #[test]
    fn ranking_precedence() {
        let test = accept_encoding!();
        assert!(test.ranked().is_empty());

        let test = accept_encoding!("gzip");
        assert_eq!(test.ranked(), vec![enc("gzip")]);

        let test = accept_encoding!("gzip;q=0.900", "*;q=0.700", "br;q=1.0");
        assert_eq!(test.ranked(), vec![enc("br"), enc("gzip"), enc("*")]);

        let test = accept_encoding!("br", "gzip", "*");
        assert_eq!(test.ranked(), vec![enc("br"), enc("gzip"), enc("*")]);

        let test = accept_encoding!("gzip", "br", "*");
        assert_eq!(test.ranked(), vec![enc("br"), enc("gzip"), enc("*")]);
    }

    #[test]
    fn preference_selection() {
        assert_eq!(accept_encoding!().preference(), Some(Preference::Any));

        assert_eq!(accept_encoding!("identity;q=0").preference(), None);
        assert_eq!(accept_encoding!("*;q=0").preference(), None);
        assert_eq!(accept_encoding!("compress;q=0", "*;q=0").preference(), None);
        assert_eq!(accept_encoding!("identity;q=0", "*;q=0").preference(), None);

        let test = accept_encoding!("*;q=0.5");
        assert_eq!(test.preference().unwrap(), enc("*"));

        let test = accept_encoding!("br;q=0");
        assert_eq!(test.preference().unwrap(), enc("identity"));

        let test = accept_encoding!("br;q=0.900", "gzip;q=1.0", "*;q=0.500");
        assert_eq!(test.preference().unwrap(), enc("gzip"));

        let test = accept_encoding!("br", "gzip", "*");
        assert_eq!(test.preference().unwrap(), enc("br"));

        let test = accept_encoding!("gzip", "br", "*");
        assert_eq!(test.preference().unwrap(), enc("br"));
    }
}
