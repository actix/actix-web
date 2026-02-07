use std::{fmt, str};

use super::common_header;
use crate::http::header;

common_header! {
    /// `Cache-Control` header, defined
    /// in [RFC 7234 §5.2](https://datatracker.ietf.org/doc/html/rfc7234#section-5.2).
    ///
    /// The `Cache-Control` header field is used to specify directives for
    /// caches along the request/response chain.  Such cache directives are
    /// unidirectional in that the presence of a directive in a request does
    /// not imply that the same directive is to be given in the response.
    ///
    /// # ABNF
    /// ```text
    /// Cache-Control   = 1#cache-directive
    /// cache-directive = token [ "=" ( token / quoted-string ) ]
    /// ```
    ///
    /// # Example Values
    /// * `no-cache`
    /// * `private, community="UCI"`
    /// * `max-age=30`
    ///
    /// # Examples
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{CacheControl, CacheDirective};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(CacheControl(vec![CacheDirective::MaxAge(86400u32)]));
    /// ```
    ///
    /// ```
    /// use actix_web::HttpResponse;
    /// use actix_web::http::header::{CacheControl, CacheDirective};
    ///
    /// let mut builder = HttpResponse::Ok();
    /// builder.insert_header(CacheControl(vec![
    ///     CacheDirective::NoCache,
    ///     CacheDirective::Private,
    ///     CacheDirective::MaxAge(360u32),
    ///     CacheDirective::Extension("foo".to_owned(), Some("bar".to_owned())),
    /// ]));
    /// ```
    (CacheControl, header::CACHE_CONTROL) => (CacheDirective)+

    test_parse_and_format {
        common_header_test!(no_headers, [b""; 0], None);
        common_header_test!(empty_header, [b""; 1], None);
        common_header_test!(bad_syntax, [b"foo="], None);

        common_header_test!(
            multiple_headers,
            [&b"no-cache"[..], &b"private"[..]],
            Some(CacheControl(vec![
                CacheDirective::NoCache,
                CacheDirective::Private,
            ]))
        );

        common_header_test!(
            argument,
            [b"max-age=100, private"],
            Some(CacheControl(vec![
                CacheDirective::MaxAge(100),
                CacheDirective::Private,
            ]))
        );

        common_header_test!(
            extension,
            [b"foo, bar=baz"],
            Some(CacheControl(vec![
                CacheDirective::Extension("foo".to_owned(), None),
                CacheDirective::Extension("bar".to_owned(), Some("baz".to_owned())),
            ]))
        );

        #[test]
        fn parse_quote_form() {
            let req = test::TestRequest::default()
                .insert_header((header::CACHE_CONTROL, "max-age=\"200\""))
                .finish();

            assert_eq!(
                Header::parse(&req).ok(),
                Some(CacheControl(vec![CacheDirective::MaxAge(200)]))
            )
        }
    }
}

/// `CacheControl` contains a list of these directives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheDirective {
    /// "no-cache"
    NoCache,
    /// "no-store"
    NoStore,
    /// "no-transform"
    NoTransform,
    /// "only-if-cached"
    OnlyIfCached,

    // request directives
    /// "max-age=delta"
    MaxAge(u32),
    /// "max-stale=delta"
    MaxStale(u32),
    /// "min-fresh=delta"
    MinFresh(u32),

    // response directives
    /// "must-revalidate"
    MustRevalidate,
    /// "public"
    Public,
    /// "private"
    Private,
    /// "proxy-revalidate"
    ProxyRevalidate,
    /// "s-maxage=delta"
    SMaxAge(u32),

    /// Extension directives. Optionally include an argument.
    Extension(String, Option<String>),
}

impl fmt::Display for CacheDirective {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::CacheDirective::*;

        let dir_str = match self {
            NoCache => "no-cache",
            NoStore => "no-store",
            NoTransform => "no-transform",
            OnlyIfCached => "only-if-cached",

            MaxAge(secs) => return write!(f, "max-age={}", secs),
            MaxStale(secs) => return write!(f, "max-stale={}", secs),
            MinFresh(secs) => return write!(f, "min-fresh={}", secs),

            MustRevalidate => "must-revalidate",
            Public => "public",
            Private => "private",
            ProxyRevalidate => "proxy-revalidate",
            SMaxAge(secs) => return write!(f, "s-maxage={}", secs),

            Extension(name, None) => name.as_str(),
            Extension(name, Some(arg)) => return write!(f, "{}={}", name, arg),
        };

        f.write_str(dir_str)
    }
}

impl str::FromStr for CacheDirective {
    type Err = Option<<u32 as str::FromStr>::Err>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use self::CacheDirective::*;

        match s {
            "" => Err(None),

            "no-cache" => Ok(NoCache),
            "no-store" => Ok(NoStore),
            "no-transform" => Ok(NoTransform),
            "only-if-cached" => Ok(OnlyIfCached),
            "must-revalidate" => Ok(MustRevalidate),
            "public" => Ok(Public),
            "private" => Ok(Private),
            "proxy-revalidate" => Ok(ProxyRevalidate),

            _ => match s.find('=') {
                Some(idx) if idx + 1 < s.len() => {
                    match (&s[..idx], s[idx + 1..].trim_matches('"')) {
                        ("max-age", secs) => secs.parse().map(MaxAge).map_err(Some),
                        ("max-stale", secs) => secs.parse().map(MaxStale).map_err(Some),
                        ("min-fresh", secs) => secs.parse().map(MinFresh).map_err(Some),
                        ("s-maxage", secs) => secs.parse().map(SMaxAge).map_err(Some),
                        (left, right) => Ok(Extension(left.to_owned(), Some(right.to_owned()))),
                    }
                }
                Some(_) => Err(None),
                None => Ok(Extension(s.to_owned(), None)),
            },
        }
    }
}
