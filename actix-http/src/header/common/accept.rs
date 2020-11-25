use std::cmp::Ordering;

use mime::Mime;

use crate::header::{qitem, QualityItem};
use crate::http::header;

header! {
    /// `Accept` header, defined in [RFC7231](http://tools.ietf.org/html/rfc7231#section-5.3.2)
    ///
    /// The `Accept` header field can be used by user agents to specify
    /// response media types that are acceptable. Accept header fields can
    /// be used to indicate that the request is specifically limited to a
    /// small set of desired types, as in the case of a request for an
    /// in-line image
    ///
    /// # ABNF
    ///
    /// ```text
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
    /// # Example values
    /// * `audio/*; q=0.2, audio/basic`
    /// * `text/plain; q=0.5, text/html, text/x-dvi; q=0.8, text/x-c`
    ///
    /// # Examples
    /// ```rust
    /// # extern crate actix_http;
    /// extern crate mime;
    /// use actix_http::Response;
    /// use actix_http::http::header::{Accept, qitem};
    ///
    /// # fn main() {
    /// let mut builder = Response::Ok();
    ///
    /// builder.set(
    ///     Accept(vec![
    ///         qitem(mime::TEXT_HTML),
    ///     ])
    /// );
    /// # }
    /// ```
    ///
    /// ```rust
    /// # extern crate actix_http;
    /// extern crate mime;
    /// use actix_http::Response;
    /// use actix_http::http::header::{Accept, qitem};
    ///
    /// # fn main() {
    /// let mut builder = Response::Ok();
    ///
    /// builder.set(
    ///     Accept(vec![
    ///         qitem(mime::APPLICATION_JSON),
    ///     ])
    /// );
    /// # }
    /// ```
    ///
    /// ```rust
    /// # extern crate actix_http;
    /// extern crate mime;
    /// use actix_http::Response;
    /// use actix_http::http::header::{Accept, QualityItem, q, qitem};
    ///
    /// # fn main() {
    /// let mut builder = Response::Ok();
    ///
    /// builder.set(
    ///     Accept(vec![
    ///         qitem(mime::TEXT_HTML),
    ///         qitem("application/xhtml+xml".parse().unwrap()),
    ///         QualityItem::new(
    ///             mime::TEXT_XML,
    ///             q(900)
    ///         ),
    ///         qitem("image/webp".parse().unwrap()),
    ///         QualityItem::new(
    ///             mime::STAR_STAR,
    ///             q(800)
    ///         ),
    ///     ])
    /// );
    /// # }
    /// ```
    (Accept, header::ACCEPT) => (QualityItem<Mime>)+

    test_accept {
        // Tests from the RFC
         test_header!(
            test1,
            vec![b"audio/*; q=0.2, audio/basic"],
            Some(Accept(vec![
                QualityItem::new("audio/*".parse().unwrap(), q(200)),
                qitem("audio/basic".parse().unwrap()),
                ])));
        test_header!(
            test2,
            vec![b"text/plain; q=0.5, text/html, text/x-dvi; q=0.8, text/x-c"],
            Some(Accept(vec![
                QualityItem::new(mime::TEXT_PLAIN, q(500)),
                qitem(mime::TEXT_HTML),
                QualityItem::new(
                    "text/x-dvi".parse().unwrap(),
                    q(800)),
                qitem("text/x-c".parse().unwrap()),
                ])));
        // Custom tests
        test_header!(
            test3,
            vec![b"text/plain; charset=utf-8"],
            Some(Accept(vec![
                qitem(mime::TEXT_PLAIN_UTF_8),
                ])));
        test_header!(
            test4,
            vec![b"text/plain; charset=utf-8; q=0.5"],
            Some(Accept(vec![
                QualityItem::new(mime::TEXT_PLAIN_UTF_8,
                    q(500)),
            ])));

        #[test]
        fn test_fuzzing1() {
            use crate::test::TestRequest;
            let req = TestRequest::with_header(crate::header::ACCEPT, "chunk#;e").finish();
            let header = Accept::parse(&req);
            assert!(header.is_ok());
        }
    }
}

impl Accept {
    /// Construct `Accept: */*`.
    pub fn star() -> Accept {
        Accept(vec![qitem(mime::STAR_STAR)])
    }

    /// Construct `Accept: application/json`.
    pub fn json() -> Accept {
        Accept(vec![qitem(mime::APPLICATION_JSON)])
    }

    /// Construct `Accept: text/*`.
    pub fn text() -> Accept {
        Accept(vec![qitem(mime::TEXT_STAR)])
    }

    /// Construct `Accept: image/*`.
    pub fn image() -> Accept {
        Accept(vec![qitem(mime::IMAGE_STAR)])
    }

    /// Construct `Accept: text/html`.
    pub fn html() -> Accept {
        Accept(vec![qitem(mime::TEXT_HTML)])
    }

    /// Returns a sorted list of mime types from highest to lowest preference, accounting for
    /// [q-factor weighting] and specificity.
    ///
    /// [q-factor weighting]: https://tools.ietf.org/html/rfc7231#section-5.3.2
    pub fn mime_precedence(&self) -> Vec<Mime> {
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

    /// Extracts the most preferable mime type, accounting for [q-factor weighting].
    ///
    /// If no q-factors are provided, the first mime type is chosen. Note that items without
    /// q-factors are given the maximum preference value.
    ///
    /// Returns `None` if contained list is empty.
    ///
    /// [q-factor weighting]: https://tools.ietf.org/html/rfc7231#section-5.3.2
    pub fn mime_preference(&self) -> Option<Mime> {
        let types = self.mime_precedence();
        types.first().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::q;

    #[test]
    fn test_mime_precedence() {
        let test = Accept(vec![]);
        assert!(test.mime_precedence().is_empty());

        let test = Accept(vec![qitem(mime::APPLICATION_JSON)]);
        assert_eq!(test.mime_precedence(), vec!(mime::APPLICATION_JSON));

        let test = Accept(vec![
            qitem(mime::TEXT_HTML),
            "application/xhtml+xml".parse().unwrap(),
            QualityItem::new("application/xml".parse().unwrap(), q(0.9)),
            QualityItem::new(mime::STAR_STAR, q(0.8)),
        ]);
        assert_eq!(
            test.mime_precedence(),
            vec![
                mime::TEXT_HTML,
                "application/xhtml+xml".parse().unwrap(),
                "application/xml".parse().unwrap(),
                mime::STAR_STAR,
            ]
        );

        let test = Accept(vec![
            qitem(mime::STAR_STAR),
            qitem(mime::IMAGE_STAR),
            qitem(mime::IMAGE_PNG),
        ]);
        assert_eq!(
            test.mime_precedence(),
            vec![mime::IMAGE_PNG, mime::IMAGE_STAR, mime::STAR_STAR]
        );
    }

    #[test]
    fn test_mime_preference() {
        let test = Accept(vec![
            qitem(mime::TEXT_HTML),
            "application/xhtml+xml".parse().unwrap(),
            QualityItem::new("application/xml".parse().unwrap(), q(0.9)),
            QualityItem::new(mime::STAR_STAR, q(0.8)),
        ]);
        assert_eq!(test.mime_preference(), Some(mime::TEXT_HTML));

        let test = Accept(vec![
            QualityItem::new("video/*".parse().unwrap(), q(0.8)),
            qitem(mime::IMAGE_PNG),
            QualityItem::new(mime::STAR_STAR, q(0.5)),
            qitem(mime::IMAGE_SVG),
            QualityItem::new(mime::IMAGE_STAR, q(0.8)),
        ]);
        assert_eq!(test.mime_preference(), Some(mime::IMAGE_PNG));
    }
}
