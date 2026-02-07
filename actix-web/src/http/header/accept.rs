use std::cmp::Ordering;

use mime::Mime;

use super::{common_header, QualityItem};
use crate::http::header;

common_header! {
    /// `Accept` header, defined in [RFC 7231 §5.3.2].
    ///
    /// The `Accept` header field can be used by user agents to specify
    /// response media types that are acceptable. Accept header fields can
    /// be used to indicate that the request is specifically limited to a
    /// small set of desired types, as in the case of a request for an
    /// in-line image
    ///
    /// # ABNF
    /// ```plain
    /// Accept = #( media-range [ accept-params ] )
    ///
    /// media-range    = ( "*/*"
    ///                  / ( type "/" "*" )
    ///                  / ( type "/" subtype )
    ///                  ) *( OWS ";" OWS parameter )
    /// accept-params  = weight *( accept-ext )
    /// accept-ext = OWS ";" OWS token [ "=" ( token / quoted-string ) ]
    /// ```
    ///
    /// # Example Values
    /// * `audio/*; q=0.2, audio/basic`
    /// * `text/plain; q=0.5, text/html, text/x-dvi; q=0.8, text/x-c`
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{Accept, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     Accept(vec![
    ///         QualityItem::max(mime::TEXT_HTML),
    ///     ])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{Accept, QualityItem};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     Accept(vec![
    ///         QualityItem::max(mime::APPLICATION_JSON),
    ///     ])
    /// );
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{Accept, QualityItem, q};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(
    ///     Accept(vec![
    ///         QualityItem::max(mime::TEXT_HTML),
    ///         QualityItem::max("application/xhtml+xml".parse().unwrap()),
    ///         QualityItem::new(mime::TEXT_XML, q(0.9)),
    ///         QualityItem::max("image/webp".parse().unwrap()),
    ///         QualityItem::new(mime::STAR_STAR, q(0.8)),
    ///     ])
    /// );
    /// ```
    ///
    /// [RFC 7231 §5.3.2]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.2
    (Accept, header::ACCEPT) => (QualityItem<Mime>)*

    test_parse_and_format {
        // Tests from the RFC
         crate::http::header::common_header_test!(
            test1,
            [b"audio/*; q=0.2, audio/basic"],
            Some(Accept(vec![
                QualityItem::new("audio/*".parse().unwrap(), q(0.2)),
                QualityItem::max("audio/basic".parse().unwrap()),
                ])));

        crate::http::header::common_header_test!(
            test2,
            [b"text/plain; q=0.5, text/html, text/x-dvi; q=0.8, text/x-c"],
            Some(Accept(vec![
                QualityItem::new(mime::TEXT_PLAIN, q(0.5)),
                QualityItem::max(mime::TEXT_HTML),
                QualityItem::new(
                    "text/x-dvi".parse().unwrap(),
                    q(0.8)),
                QualityItem::max("text/x-c".parse().unwrap()),
                ])));

        // Custom tests
        crate::http::header::common_header_test!(
            test3,
            [b"text/plain; charset=utf-8"],
            Some(Accept(vec![
                QualityItem::max(mime::TEXT_PLAIN_UTF_8),
            ])));
        crate::http::header::common_header_test!(
            test4,
            [b"text/plain; charset=utf-8; q=0.5"],
            Some(Accept(vec![
                QualityItem::new(mime::TEXT_PLAIN_UTF_8, q(0.5)),
            ])));

        #[test]
        fn test_fuzzing1() {
            let req = test::TestRequest::default()
                .insert_header((header::ACCEPT, "chunk#;e"))
                .finish();
            let header = Accept::parse(&req);
            assert!(header.is_ok());
        }
    }
}

impl Accept {
    /// Construct `Accept: */*`.
    pub fn star() -> Accept {
        Accept(vec![QualityItem::max(mime::STAR_STAR)])
    }

    /// Construct `Accept: application/json`.
    pub fn json() -> Accept {
        Accept(vec![QualityItem::max(mime::APPLICATION_JSON)])
    }

    /// Construct `Accept: text/*`.
    pub fn text() -> Accept {
        Accept(vec![QualityItem::max(mime::TEXT_STAR)])
    }

    /// Construct `Accept: image/*`.
    pub fn image() -> Accept {
        Accept(vec![QualityItem::max(mime::IMAGE_STAR)])
    }

    /// Construct `Accept: text/html`.
    pub fn html() -> Accept {
        Accept(vec![QualityItem::max(mime::TEXT_HTML)])
    }

    // TODO: method for getting best content encoding based on q-factors, available from server side
    // and if none are acceptable return None

    /// Extracts the most preferable mime type, accounting for [q-factor weighting].
    ///
    /// If no q-factors are provided, the first mime type is chosen. Note that items without
    /// q-factors are given the maximum preference value.
    ///
    /// As per the spec, will return [`mime::STAR_STAR`] (indicating no preference) if the contained
    /// list is empty.
    ///
    /// [q-factor weighting]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.2
    pub fn preference(&self) -> Mime {
        use actix_http::header::Quality;

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

        max_item.unwrap_or(mime::STAR_STAR)
    }

    /// Returns a sorted list of mime types from highest to lowest preference, accounting for
    /// [q-factor weighting] and specificity.
    ///
    /// [q-factor weighting]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.2
    pub fn ranked(&self) -> Vec<Mime> {
        if self.is_empty() {
            return vec![];
        }

        let mut types = self.0.clone();

        // use stable sort so items with equal q-factor and specificity retain listed order
        types.sort_by(|a, b| {
            // sort by q-factor descending
            b.quality.cmp(&a.quality).then_with(|| {
                // use specificity rules on mime types with
                // same q-factor (eg. text/html > text/* > */*)

                // subtypes are not comparable if main type is star, so return
                match (a.item.type_(), b.item.type_()) {
                    (mime::STAR, mime::STAR) => return Ordering::Equal,

                    // a is sorted after b
                    (mime::STAR, _) => return Ordering::Greater,

                    // a is sorted before b
                    (_, mime::STAR) => return Ordering::Less,

                    _ => {}
                }

                // in both these match expressions, the returned ordering appears
                // inverted because sort is high-to-low ("descending") precedence
                match (a.item.subtype(), b.item.subtype()) {
                    (mime::STAR, mime::STAR) => Ordering::Equal,

                    // a is sorted after b
                    (mime::STAR, _) => Ordering::Greater,

                    // a is sorted before b
                    (_, mime::STAR) => Ordering::Less,

                    _ => Ordering::Equal,
                }
            })
        });

        types.into_iter().map(|qitem| qitem.item).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::header::q;

    #[test]
    fn ranking_precedence() {
        let test = Accept(vec![]);
        assert!(test.ranked().is_empty());

        let test = Accept(vec![QualityItem::max(mime::APPLICATION_JSON)]);
        assert_eq!(test.ranked(), vec![mime::APPLICATION_JSON]);

        let test = Accept(vec![
            QualityItem::max(mime::TEXT_HTML),
            "application/xhtml+xml".parse().unwrap(),
            QualityItem::new("application/xml".parse().unwrap(), q(0.9)),
            QualityItem::new(mime::STAR_STAR, q(0.8)),
        ]);
        assert_eq!(
            test.ranked(),
            vec![
                mime::TEXT_HTML,
                "application/xhtml+xml".parse().unwrap(),
                "application/xml".parse().unwrap(),
                mime::STAR_STAR,
            ]
        );

        let test = Accept(vec![
            QualityItem::max(mime::STAR_STAR),
            QualityItem::max(mime::IMAGE_STAR),
            QualityItem::max(mime::IMAGE_PNG),
        ]);
        assert_eq!(
            test.ranked(),
            vec![mime::IMAGE_PNG, mime::IMAGE_STAR, mime::STAR_STAR]
        );
    }

    #[test]
    fn preference_selection() {
        let test = Accept(vec![
            QualityItem::max(mime::TEXT_HTML),
            "application/xhtml+xml".parse().unwrap(),
            QualityItem::new("application/xml".parse().unwrap(), q(0.9)),
            QualityItem::new(mime::STAR_STAR, q(0.8)),
        ]);
        assert_eq!(test.preference(), mime::TEXT_HTML);

        let test = Accept(vec![
            QualityItem::new("video/*".parse().unwrap(), q(0.8)),
            QualityItem::max(mime::IMAGE_PNG),
            QualityItem::new(mime::STAR_STAR, q(0.5)),
            QualityItem::max(mime::IMAGE_SVG),
            QualityItem::new(mime::IMAGE_STAR, q(0.8)),
        ]);
        assert_eq!(test.preference(), mime::IMAGE_PNG);
    }
}
