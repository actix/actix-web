use std::fmt::{self, Write};
use std::str::FromStr;

use http::header;

use crate::header::{
    fmt_comma_delimited, from_comma_delimited, Header, IntoHeaderValue, Writer,
};

/// `Cache-Control` header, defined in [RFC7234](https://tools.ietf.org/html/rfc7234#section-5.2)
///
/// The `Cache-Control` header field is used to specify directives for
/// caches along the request/response chain.  Such cache directives are
/// unidirectional in that the presence of a directive in a request does
/// not imply that the same directive is to be given in the response.
///
/// # ABNF
///
/// ```text
/// Cache-Control   = 1#cache-directive
/// cache-directive = token [ "=" ( token / quoted-string ) ]
/// ```
///
/// # Example values
///
/// * `no-cache`
/// * `private, community="UCI"`
/// * `max-age=30`
///
/// # Examples
/// ```rust
/// use actix_http::Response;
/// use actix_http::http::header::{CacheControl, CacheDirective};
///
/// let mut builder = Response::Ok();
/// builder.set(CacheControl(vec![CacheDirective::MaxAge(86400u32)]));
/// ```
///
/// ```rust
/// use actix_http::Response;
/// use actix_http::http::header::{CacheControl, CacheDirective};
///
/// let mut builder = Response::Ok();
/// builder.set(CacheControl(vec![
///     CacheDirective::NoCache,
///     CacheDirective::Private,
///     CacheDirective::MaxAge(360u32),
///     CacheDirective::Extension("foo".to_owned(), Some("bar".to_owned())),
/// ]));
/// ```
#[derive(PartialEq, Clone, Debug)]
pub struct CacheControl(pub Vec<CacheDirective>);

__hyper__deref!(CacheControl => Vec<CacheDirective>);

//TODO: this could just be the header! macro
impl Header for CacheControl {
    fn name() -> header::HeaderName {
        header::CACHE_CONTROL
    }

    #[inline]
    fn parse<T>(msg: &T) -> Result<Self, crate::error::ParseError>
    where
        T: crate::HttpMessage,
    {
        let directives = from_comma_delimited(msg.headers().get_all(&Self::name()))?;
        if !directives.is_empty() {
            Ok(CacheControl(directives))
        } else {
            Err(crate::error::ParseError::Header)
        }
    }
}

impl fmt::Display for CacheControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_comma_delimited(f, &self[..])
    }
}

impl IntoHeaderValue for CacheControl {
    type Error = header::InvalidHeaderValue;

    fn try_into(self) -> Result<header::HeaderValue, Self::Error> {
        let mut writer = Writer::new();
        let _ = write!(&mut writer, "{}", self);
        header::HeaderValue::from_maybe_shared(writer.take())
    }
}

/// `CacheControl` contains a list of these directives.
#[derive(PartialEq, Clone, Debug)]
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
        fmt::Display::fmt(
            match *self {
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

                Extension(ref name, None) => &name[..],
                Extension(ref name, Some(ref arg)) => {
                    return write!(f, "{}={}", name, arg);
                }
            },
            f,
        )
    }
}

impl FromStr for CacheDirective {
    type Err = Option<<u32 as FromStr>::Err>;
    fn from_str(s: &str) -> Result<CacheDirective, Option<<u32 as FromStr>::Err>> {
        use self::CacheDirective::*;
        match s {
            "no-cache" => Ok(NoCache),
            "no-store" => Ok(NoStore),
            "no-transform" => Ok(NoTransform),
            "only-if-cached" => Ok(OnlyIfCached),
            "must-revalidate" => Ok(MustRevalidate),
            "public" => Ok(Public),
            "private" => Ok(Private),
            "proxy-revalidate" => Ok(ProxyRevalidate),
            "" => Err(None),
            _ => match s.find('=') {
                Some(idx) if idx + 1 < s.len() => {
                    match (&s[..idx], (&s[idx + 1..]).trim_matches('"')) {
                        ("max-age", secs) => secs.parse().map(MaxAge).map_err(Some),
                        ("max-stale", secs) => secs.parse().map(MaxStale).map_err(Some),
                        ("min-fresh", secs) => secs.parse().map(MinFresh).map_err(Some),
                        ("s-maxage", secs) => secs.parse().map(SMaxAge).map_err(Some),
                        (left, right) => {
                            Ok(Extension(left.to_owned(), Some(right.to_owned())))
                        }
                    }
                }
                Some(_) => Err(None),
                None => Ok(Extension(s.to_owned(), None)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::Header;
    use crate::test::TestRequest;

    #[test]
    fn test_parse_multiple_headers() {
        let req = TestRequest::with_header(header::CACHE_CONTROL, "no-cache, private")
            .finish();
        let cache = Header::parse(&req);
        assert_eq!(
            cache.ok(),
            Some(CacheControl(vec![
                CacheDirective::NoCache,
                CacheDirective::Private,
            ]))
        )
    }

    #[test]
    fn test_parse_argument() {
        let req =
            TestRequest::with_header(header::CACHE_CONTROL, "max-age=100, private")
                .finish();
        let cache = Header::parse(&req);
        assert_eq!(
            cache.ok(),
            Some(CacheControl(vec![
                CacheDirective::MaxAge(100),
                CacheDirective::Private,
            ]))
        )
    }

    #[test]
    fn test_parse_quote_form() {
        let req =
            TestRequest::with_header(header::CACHE_CONTROL, "max-age=\"200\"").finish();
        let cache = Header::parse(&req);
        assert_eq!(
            cache.ok(),
            Some(CacheControl(vec![CacheDirective::MaxAge(200)]))
        )
    }

    #[test]
    fn test_parse_extension() {
        let req =
            TestRequest::with_header(header::CACHE_CONTROL, "foo, bar=baz").finish();
        let cache = Header::parse(&req);
        assert_eq!(
            cache.ok(),
            Some(CacheControl(vec![
                CacheDirective::Extension("foo".to_owned(), None),
                CacheDirective::Extension("bar".to_owned(), Some("baz".to_owned())),
            ]))
        )
    }

    #[test]
    fn test_parse_bad_syntax() {
        let req = TestRequest::with_header(header::CACHE_CONTROL, "foo=").finish();
        let cache: Result<CacheControl, _> = Header::parse(&req);
        assert_eq!(cache.ok(), None)
    }
}
