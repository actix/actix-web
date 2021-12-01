use language_tags::LanguageTag;

use super::{common_header, AnyOrSome, QualityItem};
use crate::http::header;

common_header! {
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
    /// use actix_web::http::header::{AcceptLanguage, LanguageTag, qitem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptLanguage(vec![
    ///         qitem(LanguageTag::parse("en-US").unwrap())
    ///     ])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{AcceptLanguage, LanguageTag, QualityItem, q, qitem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     AcceptLanguage(vec![
    ///         qitem(LanguageTag::parse("da").unwrap()),
    ///         QualityItem::new(LanguageTag::parse("en-GB").unwrap(), q(800)),
    ///         QualityItem::new(LanguageTag::parse("en").unwrap(), q(700)),
    ///     ])
    /// );
    /// ```
    (AcceptLanguage, header::ACCEPT_LANGUAGE) => (QualityItem<AnyOrSome<LanguageTag>>)+

    test_parse_and_format {
        common_header_test!(
            example_from_rfc,
            vec![b"da, en-gb;q=0.8, en;q=0.7"]
        );

        common_header_test!(
            not_ordered_by_weight,
            vec![b"en-US, en; q=0.5, fr"],
            Some(AcceptLanguage(vec![
                qitem("en-US".parse().unwrap()),
                QualityItem::new("en".parse().unwrap(), q(500)),
                qitem("fr".parse().unwrap()),
            ]))
        );

        common_header_test!(
            has_wildcard,
            vec![b"fr-CH, fr; q=0.9, en; q=0.8, de; q=0.7, *; q=0.5"],
            Some(AcceptLanguage(vec![
                qitem("fr-CH".parse().unwrap()),
                QualityItem::new("fr".parse().unwrap(), q(900)),
                QualityItem::new("en".parse().unwrap(), q(800)),
                QualityItem::new("de".parse().unwrap(), q(700)),
                QualityItem::new("*".parse().unwrap(), q(500)),
            ]))
        );
    }
}

impl AcceptLanguage {
    /// Returns a sorted list of languages from highest to lowest precedence, accounting
    /// for [q-factor weighting].
    ///
    /// [q-factor weighting]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.2
    pub fn ranked(&self) -> Vec<AnyOrSome<LanguageTag>> {
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

    /// Extracts the most preferable language, accounting for [q-factor weighting].
    ///
    /// If no q-factors are provided, the first language is chosen. Note that items without
    /// q-factors are given the maximum preference value.
    ///
    /// As per the spec, returns [`AnyOrSome::Any`] if contained list is empty.
    ///
    /// [q-factor weighting]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.2
    pub fn preference(&self) -> AnyOrSome<LanguageTag> {
        // PERF: creating a sorted list is not necessary
        self.ranked().into_iter().next().unwrap_or(AnyOrSome::Any)
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

        let test = AcceptLanguage(vec![qitem("fr-CH".parse().unwrap())]);
        assert_eq!(test.ranked(), vec!("fr-CH".parse().unwrap()));

        let test = AcceptLanguage(vec![
            QualityItem::new("fr".parse().unwrap(), q(900)),
            QualityItem::new("fr-CH".parse().unwrap(), q(1000)),
            QualityItem::new("en".parse().unwrap(), q(800)),
            QualityItem::new("*".parse().unwrap(), q(500)),
            QualityItem::new("de".parse().unwrap(), q(700)),
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
            qitem("fr".parse().unwrap()),
            qitem("fr-CH".parse().unwrap()),
            qitem("en".parse().unwrap()),
            qitem("*".parse().unwrap()),
            qitem("de".parse().unwrap()),
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
            QualityItem::new("fr".parse().unwrap(), q(900)),
            QualityItem::new("fr-CH".parse().unwrap(), q(1000)),
            QualityItem::new("en".parse().unwrap(), q(800)),
            QualityItem::new("*".parse().unwrap(), q(500)),
            QualityItem::new("de".parse().unwrap(), q(700)),
        ]);
        assert_eq!(test.preference(), AnyOrSome::Item("fr-CH".parse().unwrap()));

        let test = AcceptLanguage(vec![
            qitem("fr".parse().unwrap()),
            qitem("fr-CH".parse().unwrap()),
            qitem("en".parse().unwrap()),
            qitem("*".parse().unwrap()),
            qitem("de".parse().unwrap()),
        ]);
        assert_eq!(test.preference(), AnyOrSome::Item("fr".parse().unwrap()));

        let test = AcceptLanguage(vec![]);
        assert_eq!(test.preference(), AnyOrSome::Any);
    }
}
