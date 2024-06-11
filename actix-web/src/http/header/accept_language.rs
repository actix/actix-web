use language_tags::LanguageTag;

use super::{common_header, Preference, Quality, QualityItem};
use crate::http::header;

common_header! {
    /// `Accept-Language` header, defined
    /// in [RFC 7231 §5.3.5](https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.5)
    ///
    /// The `Accept-Language` header field can be used by user agents to indicate the set of natural
    /// languages that are preferred in the response.
    ///
    /// The `Accept-Language` header is defined in
    /// [RFC 7231 §5.3.5](https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.5) using language
    /// ranges defined in [RFC 4647 §2.1](https://datatracker.ietf.org/doc/html/rfc4647#section-2.1).
    ///
    /// # ABNF
    /// ```plain
    /// Accept-Language = 1#( language-range [ weight ] )
    /// language-range  = (1*8ALPHA *("-" 1*8alphanum)) / "*"
    /// alphanum        = ALPHA / DIGIT
    /// weight          = OWS ";" OWS "q=" qvalue
    /// qvalue          = ( "0" [ "." 0*3DIGIT ] )
    ///                 / ( "1" [ "." 0*3("0") ] )
    /// ```
    ///
    /// # Example Values
    /// - `da, en-gb;q=0.8, en;q=0.7`
    /// - `en-us;q=1.0, en;q=0.5, fr`
    /// - `fr-CH, fr;q=0.9, en;q=0.8, de;q=0.7, *;q=0.5`
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptLanguage, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptLanguage(vec![
    ///         "en-US".parse().unwrap(),
    ///     ])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptLanguage, QualityItem, q};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptLanguage(vec![
    ///         "da".parse().unwrap(),
    ///         "en-GB;q=0.8".parse().unwrap(),
    ///         "en;q=0.7".parse().unwrap(),
    ///     ])
    /// );
    /// ```
    (AcceptLanguage, header::ACCEPT_LANGUAGE) => (QualityItem<Preference<LanguageTag>>)*

    test_parse_and_format {
        common_header_test!(no_headers, [b""; 0], Some(AcceptLanguage(vec![])));

        common_header_test!(empty_header, [b""; 1], Some(AcceptLanguage(vec![])));

        common_header_test!(
            example_from_rfc,
            [b"da, en-gb;q=0.8, en;q=0.7"]
        );


        common_header_test!(
            not_ordered_by_weight,
            [b"en-US, en; q=0.5, fr"],
            Some(AcceptLanguage(vec![
                QualityItem::max("en-US".parse().unwrap()),
                QualityItem::new("en".parse().unwrap(), q(0.5)),
                QualityItem::max("fr".parse().unwrap()),
            ]))
        );

        common_header_test!(
            has_wildcard,
            [b"fr-CH, fr; q=0.9, en; q=0.8, de; q=0.7, *; q=0.5"],
            Some(AcceptLanguage(vec![
                QualityItem::max("fr-CH".parse().unwrap()),
                QualityItem::new("fr".parse().unwrap(), q(0.9)),
                QualityItem::new("en".parse().unwrap(), q(0.8)),
                QualityItem::new("de".parse().unwrap(), q(0.7)),
                QualityItem::new("*".parse().unwrap(), q(0.5)),
            ]))
        );
    }
}

impl AcceptLanguage {
    /// Extracts the most preferable language, accounting for [q-factor weighting].
    ///
    /// If no q-factors are provided, the first language is chosen. Note that items without
    /// q-factors are given the maximum preference value.
    ///
    /// As per the spec, returns [`Preference::Any`] if contained list is empty.
    ///
    /// [q-factor weighting]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.2
    pub fn preference(&self) -> Preference<LanguageTag> {
        let mut max_item = None;
        let mut max_pref = Quality::ZERO;

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

    /// Returns a sorted list of languages from highest to lowest precedence, accounting
    /// for [q-factor weighting].
    ///
    /// [q-factor weighting]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.2
    pub fn ranked(&self) -> Vec<Preference<LanguageTag>> {
        if self.0.is_empty() {
            return vec![];
        }

        let mut types = self.0.clone();

        // use stable sort so items with equal q-factor retain listed order
        types.sort_by(|a, b| {
            // sort by q-factor descending
            b.quality.cmp(&a.quality)
        });

        types.into_iter().map(|q_item| q_item.item).collect()
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

        let test = AcceptLanguage(vec![QualityItem::max("fr-CH".parse().unwrap())]);
        assert_eq!(test.ranked(), vec!["fr-CH".parse().unwrap()]);

        let test = AcceptLanguage(vec![
            QualityItem::new("fr".parse().unwrap(), q(0.900)),
            QualityItem::new("fr-CH".parse().unwrap(), q(1.0)),
            QualityItem::new("en".parse().unwrap(), q(0.800)),
            QualityItem::new("*".parse().unwrap(), q(0.500)),
            QualityItem::new("de".parse().unwrap(), q(0.700)),
        ]);
        assert_eq!(
            test.ranked(),
            vec![
                "fr-CH".parse().unwrap(),
                "fr".parse().unwrap(),
                "en".parse().unwrap(),
                "de".parse().unwrap(),
                "*".parse().unwrap(),
            ]
        );

        let test = AcceptLanguage(vec![
            QualityItem::max("fr".parse().unwrap()),
            QualityItem::max("fr-CH".parse().unwrap()),
            QualityItem::max("en".parse().unwrap()),
            QualityItem::max("*".parse().unwrap()),
            QualityItem::max("de".parse().unwrap()),
        ]);
        assert_eq!(
            test.ranked(),
            vec![
                "fr".parse().unwrap(),
                "fr-CH".parse().unwrap(),
                "en".parse().unwrap(),
                "*".parse().unwrap(),
                "de".parse().unwrap(),
            ]
        );
    }

    #[test]
    fn preference_selection() {
        let test = AcceptLanguage(vec![
            QualityItem::new("fr".parse().unwrap(), q(0.900)),
            QualityItem::new("fr-CH".parse().unwrap(), q(1.0)),
            QualityItem::new("en".parse().unwrap(), q(0.800)),
            QualityItem::new("*".parse().unwrap(), q(0.500)),
            QualityItem::new("de".parse().unwrap(), q(0.700)),
        ]);
        assert_eq!(
            test.preference(),
            Preference::Specific("fr-CH".parse().unwrap())
        );

        let test = AcceptLanguage(vec![
            QualityItem::max("fr".parse().unwrap()),
            QualityItem::max("fr-CH".parse().unwrap()),
            QualityItem::max("en".parse().unwrap()),
            QualityItem::max("*".parse().unwrap()),
            QualityItem::max("de".parse().unwrap()),
        ]);
        assert_eq!(
            test.preference(),
            Preference::Specific("fr".parse().unwrap())
        );

        let test = AcceptLanguage(vec![]);
        assert_eq!(test.preference(), Preference::Any);
    }
}
