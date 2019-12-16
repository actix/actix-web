use std::borrow::Cow;
use std::cmp;
use std::convert::From;
use std::error::Error;
use std::fmt;
use std::str::Utf8Error;

use chrono::Duration;
use percent_encoding::percent_decode;

use super::{Cookie, CookieStr, SameSite};

/// Enum corresponding to a parsing error.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ParseError {
    /// The cookie did not contain a name/value pair.
    MissingPair,
    /// The cookie's name was empty.
    EmptyName,
    /// Decoding the cookie's name or value resulted in invalid UTF-8.
    Utf8Error(Utf8Error),
    /// It is discouraged to exhaustively match on this enum as its variants may
    /// grow without a breaking-change bump in version numbers.
    #[doc(hidden)]
    __Nonexhasutive,
}

impl ParseError {
    /// Returns a description of this error as a string
    pub fn as_str(&self) -> &'static str {
        match *self {
            ParseError::MissingPair => "the cookie is missing a name/value pair",
            ParseError::EmptyName => "the cookie's name is empty",
            ParseError::Utf8Error(_) => {
                "decoding the cookie's name or value resulted in invalid UTF-8"
            }
            ParseError::__Nonexhasutive => unreachable!("__Nonexhasutive ParseError"),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<Utf8Error> for ParseError {
    fn from(error: Utf8Error) -> ParseError {
        ParseError::Utf8Error(error)
    }
}

impl Error for ParseError {}

fn indexes_of(needle: &str, haystack: &str) -> Option<(usize, usize)> {
    let haystack_start = haystack.as_ptr() as usize;
    let needle_start = needle.as_ptr() as usize;

    if needle_start < haystack_start {
        return None;
    }

    if (needle_start + needle.len()) > (haystack_start + haystack.len()) {
        return None;
    }

    let start = needle_start - haystack_start;
    let end = start + needle.len();
    Some((start, end))
}

fn name_val_decoded(
    name: &str,
    val: &str,
) -> Result<(CookieStr, CookieStr), ParseError> {
    let decoded_name = percent_decode(name.as_bytes()).decode_utf8()?;
    let decoded_value = percent_decode(val.as_bytes()).decode_utf8()?;
    let name = CookieStr::Concrete(Cow::Owned(decoded_name.into_owned()));
    let val = CookieStr::Concrete(Cow::Owned(decoded_value.into_owned()));

    Ok((name, val))
}

// This function does the real parsing but _does not_ set the `cookie_string` in
// the returned cookie object. This only exists so that the borrow to `s` is
// returned at the end of the call, allowing the `cookie_string` field to be
// set in the outer `parse` function.
fn parse_inner<'c>(s: &str, decode: bool) -> Result<Cookie<'c>, ParseError> {
    let mut attributes = s.split(';');
    let key_value = match attributes.next() {
        Some(s) => s,
        _ => panic!(),
    };

    // Determine the name = val.
    let (name, value) = match key_value.find('=') {
        Some(i) => (key_value[..i].trim(), key_value[(i + 1)..].trim()),
        None => return Err(ParseError::MissingPair),
    };

    if name.is_empty() {
        return Err(ParseError::EmptyName);
    }

    // Create a cookie with all of the defaults. We'll fill things in while we
    // iterate through the parameters below.
    let (name, value) = if decode {
        name_val_decoded(name, value)?
    } else {
        let name_indexes = indexes_of(name, s).expect("name sub");
        let value_indexes = indexes_of(value, s).expect("value sub");
        let name = CookieStr::Indexed(name_indexes.0, name_indexes.1);
        let value = CookieStr::Indexed(value_indexes.0, value_indexes.1);

        (name, value)
    };

    let mut cookie = Cookie {
        name,
        value,
        cookie_string: None,
        expires: None,
        max_age: None,
        domain: None,
        path: None,
        secure: None,
        http_only: None,
        same_site: None,
    };

    for attr in attributes {
        let (key, value) = match attr.find('=') {
            Some(i) => (attr[..i].trim(), Some(attr[(i + 1)..].trim())),
            None => (attr.trim(), None),
        };

        match (&*key.to_ascii_lowercase(), value) {
            ("secure", _) => cookie.secure = Some(true),
            ("httponly", _) => cookie.http_only = Some(true),
            ("max-age", Some(v)) => {
                // See RFC 6265 Section 5.2.2, negative values indicate that the
                // earliest possible expiration time should be used, so set the
                // max age as 0 seconds.
                cookie.max_age = match v.parse() {
                    Ok(val) if val <= 0 => Some(Duration::zero()),
                    Ok(val) => {
                        // Don't panic if the max age seconds is greater than what's supported by
                        // `Duration`.
                        let val = cmp::min(val, Duration::max_value().num_seconds());
                        Some(Duration::seconds(val))
                    }
                    Err(_) => continue,
                };
            }
            ("domain", Some(mut domain)) if !domain.is_empty() => {
                if domain.starts_with('.') {
                    domain = &domain[1..];
                }

                let (i, j) = indexes_of(domain, s).expect("domain sub");
                cookie.domain = Some(CookieStr::Indexed(i, j));
            }
            ("path", Some(v)) => {
                let (i, j) = indexes_of(v, s).expect("path sub");
                cookie.path = Some(CookieStr::Indexed(i, j));
            }
            ("samesite", Some(v)) => {
                if v.eq_ignore_ascii_case("strict") {
                    cookie.same_site = Some(SameSite::Strict);
                } else if v.eq_ignore_ascii_case("lax") {
                    cookie.same_site = Some(SameSite::Lax);
                } else {
                    // We do nothing here, for now. When/if the `SameSite`
                    // attribute becomes standard, the spec says that we should
                    // ignore this cookie, i.e, fail to parse it, when an
                    // invalid value is passed in. The draft is at
                    // http://httpwg.org/http-extensions/draft-ietf-httpbis-cookie-same-site.html.
                }
            }
            ("expires", Some(v)) => {
                // Try strptime with three date formats according to
                // http://tools.ietf.org/html/rfc2616#section-3.3.1. Try
                // additional ones as encountered in the real world.
                let tm = time::strptime(v, "%a, %d %b %Y %H:%M:%S %Z")
                    .or_else(|_| time::strptime(v, "%A, %d-%b-%y %H:%M:%S %Z"))
                    .or_else(|_| time::strptime(v, "%a, %d-%b-%Y %H:%M:%S %Z"))
                    .or_else(|_| time::strptime(v, "%a %b %d %H:%M:%S %Y"));

                if let Ok(time) = tm {
                    cookie.expires = Some(time)
                }
            }
            _ => {
                // We're going to be permissive here. If we have no idea what
                // this is, then it's something nonstandard. We're not going to
                // store it (because it's not compliant), but we're also not
                // going to emit an error.
            }
        }
    }

    Ok(cookie)
}

pub fn parse_cookie<'c, S>(cow: S, decode: bool) -> Result<Cookie<'c>, ParseError>
where
    S: Into<Cow<'c, str>>,
{
    let s = cow.into();
    let mut cookie = parse_inner(&s, decode)?;
    cookie.cookie_string = Some(s);
    Ok(cookie)
}

#[cfg(test)]
mod tests {
    use super::{Cookie, SameSite};
    use chrono::Duration;
    use time::strptime;

    macro_rules! assert_eq_parse {
        ($string:expr, $expected:expr) => {
            let cookie = match Cookie::parse($string) {
                Ok(cookie) => cookie,
                Err(e) => panic!("Failed to parse {:?}: {:?}", $string, e),
            };

            assert_eq!(cookie, $expected);
        };
    }

    macro_rules! assert_ne_parse {
        ($string:expr, $expected:expr) => {
            let cookie = match Cookie::parse($string) {
                Ok(cookie) => cookie,
                Err(e) => panic!("Failed to parse {:?}: {:?}", $string, e),
            };

            assert_ne!(cookie, $expected);
        };
    }

    #[test]
    fn parse_same_site() {
        let expected = Cookie::build("foo", "bar")
            .same_site(SameSite::Lax)
            .finish();

        assert_eq_parse!("foo=bar; SameSite=Lax", expected);
        assert_eq_parse!("foo=bar; SameSite=lax", expected);
        assert_eq_parse!("foo=bar; SameSite=LAX", expected);
        assert_eq_parse!("foo=bar; samesite=Lax", expected);
        assert_eq_parse!("foo=bar; SAMESITE=Lax", expected);

        let expected = Cookie::build("foo", "bar")
            .same_site(SameSite::Strict)
            .finish();

        assert_eq_parse!("foo=bar; SameSite=Strict", expected);
        assert_eq_parse!("foo=bar; SameSITE=Strict", expected);
        assert_eq_parse!("foo=bar; SameSite=strict", expected);
        assert_eq_parse!("foo=bar; SameSite=STrICT", expected);
        assert_eq_parse!("foo=bar; SameSite=STRICT", expected);
    }

    #[test]
    fn parse() {
        assert!(Cookie::parse("bar").is_err());
        assert!(Cookie::parse("=bar").is_err());
        assert!(Cookie::parse(" =bar").is_err());
        assert!(Cookie::parse("foo=").is_ok());

        let expected = Cookie::build("foo", "bar=baz").finish();
        assert_eq_parse!("foo=bar=baz", expected);

        let mut expected = Cookie::build("foo", "bar").finish();
        assert_eq_parse!("foo=bar", expected);
        assert_eq_parse!("foo = bar", expected);
        assert_eq_parse!(" foo=bar ", expected);
        assert_eq_parse!(" foo=bar ;Domain=", expected);
        assert_eq_parse!(" foo=bar ;Domain= ", expected);
        assert_eq_parse!(" foo=bar ;Ignored", expected);

        let mut unexpected = Cookie::build("foo", "bar").http_only(false).finish();
        assert_ne_parse!(" foo=bar ;HttpOnly", unexpected);
        assert_ne_parse!(" foo=bar; httponly", unexpected);

        expected.set_http_only(true);
        assert_eq_parse!(" foo=bar ;HttpOnly", expected);
        assert_eq_parse!(" foo=bar ;httponly", expected);
        assert_eq_parse!(" foo=bar ;HTTPONLY=whatever", expected);
        assert_eq_parse!(" foo=bar ; sekure; HTTPONLY", expected);

        expected.set_secure(true);
        assert_eq_parse!(" foo=bar ;HttpOnly; Secure", expected);
        assert_eq_parse!(" foo=bar ;HttpOnly; Secure=aaaa", expected);

        unexpected.set_http_only(true);
        unexpected.set_secure(true);
        assert_ne_parse!(" foo=bar ;HttpOnly; skeure", unexpected);
        assert_ne_parse!(" foo=bar ;HttpOnly; =secure", unexpected);
        assert_ne_parse!(" foo=bar ;HttpOnly;", unexpected);

        unexpected.set_secure(false);
        assert_ne_parse!(" foo=bar ;HttpOnly; secure", unexpected);
        assert_ne_parse!(" foo=bar ;HttpOnly; secure", unexpected);
        assert_ne_parse!(" foo=bar ;HttpOnly; secure", unexpected);

        expected.set_max_age(Duration::zero());
        assert_eq_parse!(" foo=bar ;HttpOnly; Secure; Max-Age=0", expected);
        assert_eq_parse!(" foo=bar ;HttpOnly; Secure; Max-Age = 0 ", expected);
        assert_eq_parse!(" foo=bar ;HttpOnly; Secure; Max-Age=-1", expected);
        assert_eq_parse!(" foo=bar ;HttpOnly; Secure; Max-Age = -1 ", expected);

        expected.set_max_age(Duration::minutes(1));
        assert_eq_parse!(" foo=bar ;HttpOnly; Secure; Max-Age=60", expected);
        assert_eq_parse!(" foo=bar ;HttpOnly; Secure; Max-Age =   60 ", expected);

        expected.set_max_age(Duration::seconds(4));
        assert_eq_parse!(" foo=bar ;HttpOnly; Secure; Max-Age=4", expected);
        assert_eq_parse!(" foo=bar ;HttpOnly; Secure; Max-Age = 4 ", expected);

        unexpected.set_secure(true);
        unexpected.set_max_age(Duration::minutes(1));
        assert_ne_parse!(" foo=bar ;HttpOnly; Secure; Max-Age=122", unexpected);
        assert_ne_parse!(" foo=bar ;HttpOnly; Secure; Max-Age = 38 ", unexpected);
        assert_ne_parse!(" foo=bar ;HttpOnly; Secure; Max-Age=51", unexpected);
        assert_ne_parse!(" foo=bar ;HttpOnly; Secure; Max-Age = -1 ", unexpected);
        assert_ne_parse!(" foo=bar ;HttpOnly; Secure; Max-Age = 0", unexpected);

        expected.set_path("/");
        assert_eq_parse!("foo=bar;HttpOnly; Secure; Max-Age=4; Path=/", expected);
        assert_eq_parse!("foo=bar;HttpOnly; Secure; Max-Age=4;Path=/", expected);

        expected.set_path("/foo");
        assert_eq_parse!("foo=bar;HttpOnly; Secure; Max-Age=4; Path=/foo", expected);
        assert_eq_parse!("foo=bar;HttpOnly; Secure; Max-Age=4;Path=/foo", expected);
        assert_eq_parse!("foo=bar;HttpOnly; Secure; Max-Age=4;path=/foo", expected);
        assert_eq_parse!("foo=bar;HttpOnly; Secure; Max-Age=4;path = /foo", expected);

        unexpected.set_max_age(Duration::seconds(4));
        unexpected.set_path("/bar");
        assert_ne_parse!("foo=bar;HttpOnly; Secure; Max-Age=4; Path=/foo", unexpected);
        assert_ne_parse!("foo=bar;HttpOnly; Secure; Max-Age=4;Path=/baz", unexpected);

        expected.set_domain("www.foo.com");
        assert_eq_parse!(
            " foo=bar ;HttpOnly; Secure; Max-Age=4; Path=/foo; \
             Domain=www.foo.com",
            expected
        );

        expected.set_domain("foo.com");
        assert_eq_parse!(
            " foo=bar ;HttpOnly; Secure; Max-Age=4; Path=/foo; \
             Domain=foo.com",
            expected
        );
        assert_eq_parse!(
            " foo=bar ;HttpOnly; Secure; Max-Age=4; Path=/foo; \
             Domain=FOO.COM",
            expected
        );

        unexpected.set_path("/foo");
        unexpected.set_domain("bar.com");
        assert_ne_parse!(
            " foo=bar ;HttpOnly; Secure; Max-Age=4; Path=/foo; \
             Domain=foo.com",
            unexpected
        );
        assert_ne_parse!(
            " foo=bar ;HttpOnly; Secure; Max-Age=4; Path=/foo; \
             Domain=FOO.COM",
            unexpected
        );

        let time_str = "Wed, 21 Oct 2015 07:28:00 GMT";
        let expires = strptime(time_str, "%a, %d %b %Y %H:%M:%S %Z").unwrap();
        expected.set_expires(expires);
        assert_eq_parse!(
            " foo=bar ;HttpOnly; Secure; Max-Age=4; Path=/foo; \
             Domain=foo.com; Expires=Wed, 21 Oct 2015 07:28:00 GMT",
            expected
        );

        unexpected.set_domain("foo.com");
        let bad_expires = strptime(time_str, "%a, %d %b %Y %H:%S:%M %Z").unwrap();
        expected.set_expires(bad_expires);
        assert_ne_parse!(
            " foo=bar ;HttpOnly; Secure; Max-Age=4; Path=/foo; \
             Domain=foo.com; Expires=Wed, 21 Oct 2015 07:28:00 GMT",
            unexpected
        );
    }

    #[test]
    fn odd_characters() {
        let expected = Cookie::new("foo", "b%2Fr");
        assert_eq_parse!("foo=b%2Fr", expected);
    }

    #[test]
    fn odd_characters_encoded() {
        let expected = Cookie::new("foo", "b/r");
        let cookie = match Cookie::parse_encoded("foo=b%2Fr") {
            Ok(cookie) => cookie,
            Err(e) => panic!("Failed to parse: {:?}", e),
        };

        assert_eq!(cookie, expected);
    }

    #[test]
    fn do_not_panic_on_large_max_ages() {
        let max_seconds = Duration::max_value().num_seconds();
        let expected = Cookie::build("foo", "bar").max_age(max_seconds).finish();
        assert_eq_parse!(format!(" foo=bar; Max-Age={:?}", max_seconds + 1), expected);
    }
}
