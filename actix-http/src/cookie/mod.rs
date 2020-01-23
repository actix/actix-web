//! https://github.com/alexcrichton/cookie-rs fork
//!
//! HTTP cookie parsing and cookie jar management.
//!
//! This crates provides the [`Cookie`](struct.Cookie.html) type, which directly
//! maps to an HTTP cookie, and the [`CookieJar`](struct.CookieJar.html) type,
//! which allows for simple management of many cookies as well as encryption and
//! signing of cookies for session management.
//!
//! # Features
//!
//! This crates can be configured at compile-time through the following Cargo
//! features:
//!
//!
//! * **secure** (disabled by default)
//!
//!   Enables signed and private (signed + encrypted) cookie jars.
//!
//!   When this feature is enabled, the
//!   [signed](struct.CookieJar.html#method.signed) and
//!   [private](struct.CookieJar.html#method.private) method of `CookieJar` and
//!   [`SignedJar`](struct.SignedJar.html) and
//!   [`PrivateJar`](struct.PrivateJar.html) structures are available. The jars
//!   act as "children jars", allowing for easy retrieval and addition of signed
//!   and/or encrypted cookies to a cookie jar. When this feature is disabled,
//!   none of the types are available.
//!
//! * **percent-encode** (disabled by default)
//!
//!   Enables percent encoding and decoding of names and values in cookies.
//!
//!   When this feature is enabled, the
//!   [encoded](struct.Cookie.html#method.encoded) and
//!   [`parse_encoded`](struct.Cookie.html#method.parse_encoded) methods of
//!   `Cookie` become available. The `encoded` method returns a wrapper around a
//!   `Cookie` whose `Display` implementation percent-encodes the name and value
//!   of the cookie. The `parse_encoded` method percent-decodes the name and
//!   value of a `Cookie` during parsing. When this feature is disabled, the
//!   `encoded` and `parse_encoded` methods are not available.
//!
//! You can enable features via the `Cargo.toml` file:
//!
//! ```ignore
//! [dependencies.cookie]
//! features = ["secure", "percent-encode"]
//! ```

#![doc(html_root_url = "https://docs.rs/cookie/0.11")]
#![deny(missing_docs)]

mod builder;
mod delta;
mod draft;
mod jar;
mod parse;

#[cfg(feature = "secure-cookies")]
#[macro_use]
mod secure;
#[cfg(feature = "secure-cookies")]
pub use self::secure::*;

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use chrono::Duration;
use percent_encoding::{percent_encode, AsciiSet, CONTROLS};
use time::Tm;

pub use self::builder::CookieBuilder;
pub use self::draft::*;
pub use self::jar::{CookieJar, Delta, Iter};
use self::parse::parse_cookie;
pub use self::parse::ParseError;

/// https://url.spec.whatwg.org/#fragment-percent-encode-set
const FRAGMENT: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'<').add(b'>').add(b'`');

/// https://url.spec.whatwg.org/#path-percent-encode-set
const PATH: &AsciiSet = &FRAGMENT.add(b'#').add(b'?').add(b'{').add(b'}');

/// https://url.spec.whatwg.org/#userinfo-percent-encode-set
pub const USERINFO: &AsciiSet = &PATH
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'=')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'|');

#[derive(Debug, Clone)]
enum CookieStr {
    /// An string derived from indexes (start, end).
    Indexed(usize, usize),
    /// A string derived from a concrete string.
    Concrete(Cow<'static, str>),
}

impl CookieStr {
    /// Retrieves the string `self` corresponds to. If `self` is derived from
    /// indexes, the corresponding subslice of `string` is returned. Otherwise,
    /// the concrete string is returned.
    ///
    /// # Panics
    ///
    /// Panics if `self` is an indexed string and `string` is None.
    fn to_str<'s>(&'s self, string: Option<&'s Cow<'_, str>>) -> &'s str {
        match *self {
            CookieStr::Indexed(i, j) => {
                let s = string.expect(
                    "`Some` base string must exist when \
                     converting indexed str to str! (This is a module invariant.)",
                );
                &s[i..j]
            }
            CookieStr::Concrete(ref cstr) => &*cstr,
        }
    }

    #[allow(clippy::ptr_arg)]
    fn to_raw_str<'s, 'c: 's>(&'s self, string: &'s Cow<'c, str>) -> Option<&'c str> {
        match *self {
            CookieStr::Indexed(i, j) => match *string {
                Cow::Borrowed(s) => Some(&s[i..j]),
                Cow::Owned(_) => None,
            },
            CookieStr::Concrete(_) => None,
        }
    }
}

/// Representation of an HTTP cookie.
///
/// # Constructing a `Cookie`
///
/// To construct a cookie with only a name/value, use the [new](#method.new)
/// method:
///
/// ```rust
/// use actix_http::cookie::Cookie;
///
/// let cookie = Cookie::new("name", "value");
/// assert_eq!(&cookie.to_string(), "name=value");
/// ```
///
/// To construct more elaborate cookies, use the [build](#method.build) method
/// and [`CookieBuilder`](struct.CookieBuilder.html) methods:
///
/// ```rust
/// use actix_http::cookie::Cookie;
///
/// let cookie = Cookie::build("name", "value")
///     .domain("www.rust-lang.org")
///     .path("/")
///     .secure(true)
///     .http_only(true)
///     .finish();
/// ```
#[derive(Debug, Clone)]
pub struct Cookie<'c> {
    /// Storage for the cookie string. Only used if this structure was derived
    /// from a string that was subsequently parsed.
    cookie_string: Option<Cow<'c, str>>,
    /// The cookie's name.
    name: CookieStr,
    /// The cookie's value.
    value: CookieStr,
    /// The cookie's expiration, if any.
    expires: Option<Tm>,
    /// The cookie's maximum age, if any.
    max_age: Option<Duration>,
    /// The cookie's domain, if any.
    domain: Option<CookieStr>,
    /// The cookie's path domain, if any.
    path: Option<CookieStr>,
    /// Whether this cookie was marked Secure.
    secure: Option<bool>,
    /// Whether this cookie was marked HttpOnly.
    http_only: Option<bool>,
    /// The draft `SameSite` attribute.
    same_site: Option<SameSite>,
}

impl Cookie<'static> {
    /// Creates a new `Cookie` with the given name and value.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let cookie = Cookie::new("name", "value");
    /// assert_eq!(cookie.name_value(), ("name", "value"));
    /// ```
    pub fn new<N, V>(name: N, value: V) -> Cookie<'static>
    where
        N: Into<Cow<'static, str>>,
        V: Into<Cow<'static, str>>,
    {
        Cookie {
            cookie_string: None,
            name: CookieStr::Concrete(name.into()),
            value: CookieStr::Concrete(value.into()),
            expires: None,
            max_age: None,
            domain: None,
            path: None,
            secure: None,
            http_only: None,
            same_site: None,
        }
    }

    /// Creates a new `Cookie` with the given name and an empty value.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let cookie = Cookie::named("name");
    /// assert_eq!(cookie.name(), "name");
    /// assert!(cookie.value().is_empty());
    /// ```
    pub fn named<N>(name: N) -> Cookie<'static>
    where
        N: Into<Cow<'static, str>>,
    {
        Cookie::new(name, "")
    }

    /// Creates a new `CookieBuilder` instance from the given key and value
    /// strings.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::build("foo", "bar").finish();
    /// assert_eq!(c.name_value(), ("foo", "bar"));
    /// ```
    pub fn build<N, V>(name: N, value: V) -> CookieBuilder
    where
        N: Into<Cow<'static, str>>,
        V: Into<Cow<'static, str>>,
    {
        CookieBuilder::new(name, value)
    }
}

impl<'c> Cookie<'c> {
    /// Parses a `Cookie` from the given HTTP cookie header value string. Does
    /// not perform any percent-decoding.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::parse("foo=bar%20baz; HttpOnly").unwrap();
    /// assert_eq!(c.name_value(), ("foo", "bar%20baz"));
    /// assert_eq!(c.http_only(), Some(true));
    /// ```
    pub fn parse<S>(s: S) -> Result<Cookie<'c>, ParseError>
    where
        S: Into<Cow<'c, str>>,
    {
        parse_cookie(s, false)
    }

    /// Parses a `Cookie` from the given HTTP cookie header value string where
    /// the name and value fields are percent-encoded. Percent-decodes the
    /// name/value fields.
    ///
    /// This API requires the `percent-encode` feature to be enabled on this
    /// crate.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::parse_encoded("foo=bar%20baz; HttpOnly").unwrap();
    /// assert_eq!(c.name_value(), ("foo", "bar baz"));
    /// assert_eq!(c.http_only(), Some(true));
    /// ```
    pub fn parse_encoded<S>(s: S) -> Result<Cookie<'c>, ParseError>
    where
        S: Into<Cow<'c, str>>,
    {
        parse_cookie(s, true)
    }

    /// Wraps `self` in an `EncodedCookie`: a cost-free wrapper around `Cookie`
    /// whose `Display` implementation percent-encodes the name and value of the
    /// wrapped `Cookie`.
    ///
    /// This method is only available when the `percent-encode` feature is
    /// enabled.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let mut c = Cookie::new("my name", "this; value?");
    /// assert_eq!(&c.encoded().to_string(), "my%20name=this%3B%20value%3F");
    /// ```
    pub fn encoded<'a>(&'a self) -> EncodedCookie<'a, 'c> {
        EncodedCookie(self)
    }

    /// Converts `self` into a `Cookie` with a static lifetime. This method
    /// results in at most one allocation.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::new("a", "b");
    /// let owned_cookie = c.into_owned();
    /// assert_eq!(owned_cookie.name_value(), ("a", "b"));
    /// ```
    pub fn into_owned(self) -> Cookie<'static> {
        Cookie {
            cookie_string: self.cookie_string.map(|s| s.into_owned().into()),
            name: self.name,
            value: self.value,
            expires: self.expires,
            max_age: self.max_age,
            domain: self.domain,
            path: self.path,
            secure: self.secure,
            http_only: self.http_only,
            same_site: self.same_site,
        }
    }

    /// Returns the name of `self`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::new("name", "value");
    /// assert_eq!(c.name(), "name");
    /// ```
    #[inline]
    pub fn name(&self) -> &str {
        self.name.to_str(self.cookie_string.as_ref())
    }

    /// Returns the value of `self`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::new("name", "value");
    /// assert_eq!(c.value(), "value");
    /// ```
    #[inline]
    pub fn value(&self) -> &str {
        self.value.to_str(self.cookie_string.as_ref())
    }

    /// Returns the name and value of `self` as a tuple of `(name, value)`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::new("name", "value");
    /// assert_eq!(c.name_value(), ("name", "value"));
    /// ```
    #[inline]
    pub fn name_value(&self) -> (&str, &str) {
        (self.name(), self.value())
    }

    /// Returns whether this cookie was marked `HttpOnly` or not. Returns
    /// `Some(true)` when the cookie was explicitly set (manually or parsed) as
    /// `HttpOnly`, `Some(false)` when `http_only` was manually set to `false`,
    /// and `None` otherwise.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::parse("name=value; httponly").unwrap();
    /// assert_eq!(c.http_only(), Some(true));
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert_eq!(c.http_only(), None);
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert_eq!(c.http_only(), None);
    ///
    /// // An explicitly set "false" value.
    /// c.set_http_only(false);
    /// assert_eq!(c.http_only(), Some(false));
    ///
    /// // An explicitly set "true" value.
    /// c.set_http_only(true);
    /// assert_eq!(c.http_only(), Some(true));
    /// ```
    #[inline]
    pub fn http_only(&self) -> Option<bool> {
        self.http_only
    }

    /// Returns whether this cookie was marked `Secure` or not. Returns
    /// `Some(true)` when the cookie was explicitly set (manually or parsed) as
    /// `Secure`, `Some(false)` when `secure` was manually set to `false`, and
    /// `None` otherwise.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::parse("name=value; Secure").unwrap();
    /// assert_eq!(c.secure(), Some(true));
    ///
    /// let mut c = Cookie::parse("name=value").unwrap();
    /// assert_eq!(c.secure(), None);
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert_eq!(c.secure(), None);
    ///
    /// // An explicitly set "false" value.
    /// c.set_secure(false);
    /// assert_eq!(c.secure(), Some(false));
    ///
    /// // An explicitly set "true" value.
    /// c.set_secure(true);
    /// assert_eq!(c.secure(), Some(true));
    /// ```
    #[inline]
    pub fn secure(&self) -> Option<bool> {
        self.secure
    }

    /// Returns the `SameSite` attribute of this cookie if one was specified.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::{Cookie, SameSite};
    ///
    /// let c = Cookie::parse("name=value; SameSite=Lax").unwrap();
    /// assert_eq!(c.same_site(), Some(SameSite::Lax));
    /// ```
    #[inline]
    pub fn same_site(&self) -> Option<SameSite> {
        self.same_site
    }

    /// Returns the specified max-age of the cookie if one was specified.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::parse("name=value").unwrap();
    /// assert_eq!(c.max_age(), None);
    ///
    /// let c = Cookie::parse("name=value; Max-Age=3600").unwrap();
    /// assert_eq!(c.max_age().map(|age| age.num_hours()), Some(1));
    /// ```
    #[inline]
    pub fn max_age(&self) -> Option<Duration> {
        self.max_age
    }

    /// Returns the `Path` of the cookie if one was specified.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::parse("name=value").unwrap();
    /// assert_eq!(c.path(), None);
    ///
    /// let c = Cookie::parse("name=value; Path=/").unwrap();
    /// assert_eq!(c.path(), Some("/"));
    ///
    /// let c = Cookie::parse("name=value; path=/sub").unwrap();
    /// assert_eq!(c.path(), Some("/sub"));
    /// ```
    #[inline]
    pub fn path(&self) -> Option<&str> {
        match self.path {
            Some(ref c) => Some(c.to_str(self.cookie_string.as_ref())),
            None => None,
        }
    }

    /// Returns the `Domain` of the cookie if one was specified.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::parse("name=value").unwrap();
    /// assert_eq!(c.domain(), None);
    ///
    /// let c = Cookie::parse("name=value; Domain=crates.io").unwrap();
    /// assert_eq!(c.domain(), Some("crates.io"));
    /// ```
    #[inline]
    pub fn domain(&self) -> Option<&str> {
        match self.domain {
            Some(ref c) => Some(c.to_str(self.cookie_string.as_ref())),
            None => None,
        }
    }

    /// Returns the `Expires` time of the cookie if one was specified.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::parse("name=value").unwrap();
    /// assert_eq!(c.expires(), None);
    ///
    /// let expire_time = "Wed, 21 Oct 2017 07:28:00 GMT";
    /// let cookie_str = format!("name=value; Expires={}", expire_time);
    /// let c = Cookie::parse(cookie_str).unwrap();
    /// assert_eq!(c.expires().map(|t| t.tm_year), Some(117));
    /// ```
    #[inline]
    pub fn expires(&self) -> Option<Tm> {
        self.expires
    }

    /// Sets the name of `self` to `name`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert_eq!(c.name(), "name");
    ///
    /// c.set_name("foo");
    /// assert_eq!(c.name(), "foo");
    /// ```
    pub fn set_name<N: Into<Cow<'static, str>>>(&mut self, name: N) {
        self.name = CookieStr::Concrete(name.into())
    }

    /// Sets the value of `self` to `value`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert_eq!(c.value(), "value");
    ///
    /// c.set_value("bar");
    /// assert_eq!(c.value(), "bar");
    /// ```
    pub fn set_value<V: Into<Cow<'static, str>>>(&mut self, value: V) {
        self.value = CookieStr::Concrete(value.into())
    }

    /// Sets the value of `http_only` in `self` to `value`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert_eq!(c.http_only(), None);
    ///
    /// c.set_http_only(true);
    /// assert_eq!(c.http_only(), Some(true));
    /// ```
    #[inline]
    pub fn set_http_only(&mut self, value: bool) {
        self.http_only = Some(value);
    }

    /// Sets the value of `secure` in `self` to `value`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert_eq!(c.secure(), None);
    ///
    /// c.set_secure(true);
    /// assert_eq!(c.secure(), Some(true));
    /// ```
    #[inline]
    pub fn set_secure(&mut self, value: bool) {
        self.secure = Some(value);
    }

    /// Sets the value of `same_site` in `self` to `value`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::{Cookie, SameSite};
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert!(c.same_site().is_none());
    ///
    /// c.set_same_site(SameSite::Strict);
    /// assert_eq!(c.same_site(), Some(SameSite::Strict));
    /// ```
    #[inline]
    pub fn set_same_site(&mut self, value: SameSite) {
        self.same_site = Some(value);
    }

    /// Sets the value of `max_age` in `self` to `value`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    /// use chrono::Duration;
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert_eq!(c.max_age(), None);
    ///
    /// c.set_max_age(Duration::hours(10));
    /// assert_eq!(c.max_age(), Some(Duration::hours(10)));
    /// ```
    #[inline]
    pub fn set_max_age(&mut self, value: Duration) {
        self.max_age = Some(value);
    }

    /// Sets the `path` of `self` to `path`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert_eq!(c.path(), None);
    ///
    /// c.set_path("/");
    /// assert_eq!(c.path(), Some("/"));
    /// ```
    pub fn set_path<P: Into<Cow<'static, str>>>(&mut self, path: P) {
        self.path = Some(CookieStr::Concrete(path.into()));
    }

    /// Sets the `domain` of `self` to `domain`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert_eq!(c.domain(), None);
    ///
    /// c.set_domain("rust-lang.org");
    /// assert_eq!(c.domain(), Some("rust-lang.org"));
    /// ```
    pub fn set_domain<D: Into<Cow<'static, str>>>(&mut self, domain: D) {
        self.domain = Some(CookieStr::Concrete(domain.into()));
    }

    /// Sets the expires field of `self` to `time`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let mut c = Cookie::new("name", "value");
    /// assert_eq!(c.expires(), None);
    ///
    /// let mut now = time::now();
    /// now.tm_year += 1;
    ///
    /// c.set_expires(now);
    /// assert!(c.expires().is_some())
    /// ```
    #[inline]
    pub fn set_expires(&mut self, time: Tm) {
        self.expires = Some(time);
    }

    /// Makes `self` a "permanent" cookie by extending its expiration and max
    /// age 20 years into the future.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    /// use chrono::Duration;
    ///
    /// let mut c = Cookie::new("foo", "bar");
    /// assert!(c.expires().is_none());
    /// assert!(c.max_age().is_none());
    ///
    /// c.make_permanent();
    /// assert!(c.expires().is_some());
    /// assert_eq!(c.max_age(), Some(Duration::days(365 * 20)));
    /// ```
    pub fn make_permanent(&mut self) {
        let twenty_years = Duration::days(365 * 20);
        self.set_max_age(twenty_years);
        self.set_expires(time::now() + twenty_years);
    }

    fn fmt_parameters(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(true) = self.http_only() {
            write!(f, "; HttpOnly")?;
        }

        if let Some(true) = self.secure() {
            write!(f, "; Secure")?;
        }

        if let Some(same_site) = self.same_site() {
            write!(f, "; SameSite={}", same_site)?;
        }

        if let Some(path) = self.path() {
            write!(f, "; Path={}", path)?;
        }

        if let Some(domain) = self.domain() {
            write!(f, "; Domain={}", domain)?;
        }

        if let Some(max_age) = self.max_age() {
            write!(f, "; Max-Age={}", max_age.num_seconds())?;
        }

        if let Some(time) = self.expires() {
            write!(f, "; Expires={}", time.rfc822())?;
        }

        Ok(())
    }

    /// Returns the name of `self` as a string slice of the raw string `self`
    /// was originally parsed from. If `self` was not originally parsed from a
    /// raw string, returns `None`.
    ///
    /// This method differs from [name](#method.name) in that it returns a
    /// string with the same lifetime as the originally parsed string. This
    /// lifetime may outlive `self`. If a longer lifetime is not required, or
    /// you're unsure if you need a longer lifetime, use [name](#method.name).
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let cookie_string = format!("{}={}", "foo", "bar");
    ///
    /// // `c` will be dropped at the end of the scope, but `name` will live on
    /// let name = {
    ///     let c = Cookie::parse(cookie_string.as_str()).unwrap();
    ///     c.name_raw()
    /// };
    ///
    /// assert_eq!(name, Some("foo"));
    /// ```
    #[inline]
    pub fn name_raw(&self) -> Option<&'c str> {
        self.cookie_string
            .as_ref()
            .and_then(|s| self.name.to_raw_str(s))
    }

    /// Returns the value of `self` as a string slice of the raw string `self`
    /// was originally parsed from. If `self` was not originally parsed from a
    /// raw string, returns `None`.
    ///
    /// This method differs from [value](#method.value) in that it returns a
    /// string with the same lifetime as the originally parsed string. This
    /// lifetime may outlive `self`. If a longer lifetime is not required, or
    /// you're unsure if you need a longer lifetime, use [value](#method.value).
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let cookie_string = format!("{}={}", "foo", "bar");
    ///
    /// // `c` will be dropped at the end of the scope, but `value` will live on
    /// let value = {
    ///     let c = Cookie::parse(cookie_string.as_str()).unwrap();
    ///     c.value_raw()
    /// };
    ///
    /// assert_eq!(value, Some("bar"));
    /// ```
    #[inline]
    pub fn value_raw(&self) -> Option<&'c str> {
        self.cookie_string
            .as_ref()
            .and_then(|s| self.value.to_raw_str(s))
    }

    /// Returns the `Path` of `self` as a string slice of the raw string `self`
    /// was originally parsed from. If `self` was not originally parsed from a
    /// raw string, or if `self` doesn't contain a `Path`, or if the `Path` has
    /// changed since parsing, returns `None`.
    ///
    /// This method differs from [path](#method.path) in that it returns a
    /// string with the same lifetime as the originally parsed string. This
    /// lifetime may outlive `self`. If a longer lifetime is not required, or
    /// you're unsure if you need a longer lifetime, use [path](#method.path).
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let cookie_string = format!("{}={}; Path=/", "foo", "bar");
    ///
    /// // `c` will be dropped at the end of the scope, but `path` will live on
    /// let path = {
    ///     let c = Cookie::parse(cookie_string.as_str()).unwrap();
    ///     c.path_raw()
    /// };
    ///
    /// assert_eq!(path, Some("/"));
    /// ```
    #[inline]
    pub fn path_raw(&self) -> Option<&'c str> {
        match (self.path.as_ref(), self.cookie_string.as_ref()) {
            (Some(path), Some(string)) => path.to_raw_str(string),
            _ => None,
        }
    }

    /// Returns the `Domain` of `self` as a string slice of the raw string
    /// `self` was originally parsed from. If `self` was not originally parsed
    /// from a raw string, or if `self` doesn't contain a `Domain`, or if the
    /// `Domain` has changed since parsing, returns `None`.
    ///
    /// This method differs from [domain](#method.domain) in that it returns a
    /// string with the same lifetime as the originally parsed string. This
    /// lifetime may outlive `self` struct. If a longer lifetime is not
    /// required, or you're unsure if you need a longer lifetime, use
    /// [domain](#method.domain).
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let cookie_string = format!("{}={}; Domain=crates.io", "foo", "bar");
    ///
    /// //`c` will be dropped at the end of the scope, but `domain` will live on
    /// let domain = {
    ///     let c = Cookie::parse(cookie_string.as_str()).unwrap();
    ///     c.domain_raw()
    /// };
    ///
    /// assert_eq!(domain, Some("crates.io"));
    /// ```
    #[inline]
    pub fn domain_raw(&self) -> Option<&'c str> {
        match (self.domain.as_ref(), self.cookie_string.as_ref()) {
            (Some(domain), Some(string)) => domain.to_raw_str(string),
            _ => None,
        }
    }
}

/// Wrapper around `Cookie` whose `Display` implementation percent-encodes the
/// cookie's name and value.
///
/// A value of this type can be obtained via the
/// [encoded](struct.Cookie.html#method.encoded) method on
/// [Cookie](struct.Cookie.html). This type should only be used for its
/// `Display` implementation.
///
/// This type is only available when the `percent-encode` feature is enabled.
///
/// # Example
///
/// ```rust
/// use actix_http::cookie::Cookie;
///
/// let mut c = Cookie::new("my name", "this; value?");
/// assert_eq!(&c.encoded().to_string(), "my%20name=this%3B%20value%3F");
/// ```
pub struct EncodedCookie<'a, 'c>(&'a Cookie<'c>);

impl<'a, 'c: 'a> fmt::Display for EncodedCookie<'a, 'c> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Percent-encode the name and value.
        let name = percent_encode(self.0.name().as_bytes(), USERINFO);
        let value = percent_encode(self.0.value().as_bytes(), USERINFO);

        // Write out the name/value pair and the cookie's parameters.
        write!(f, "{}={}", name, value)?;
        self.0.fmt_parameters(f)
    }
}

impl<'c> fmt::Display for Cookie<'c> {
    /// Formats the cookie `self` as a `Set-Cookie` header value.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let mut cookie = Cookie::build("foo", "bar")
    ///     .path("/")
    ///     .finish();
    ///
    /// assert_eq!(&cookie.to_string(), "foo=bar; Path=/");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.name(), self.value())?;
        self.fmt_parameters(f)
    }
}

impl FromStr for Cookie<'static> {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Cookie<'static>, ParseError> {
        Cookie::parse(s).map(|c| c.into_owned())
    }
}

impl<'a, 'b> PartialEq<Cookie<'b>> for Cookie<'a> {
    fn eq(&self, other: &Cookie<'b>) -> bool {
        let so_far_so_good = self.name() == other.name()
            && self.value() == other.value()
            && self.http_only() == other.http_only()
            && self.secure() == other.secure()
            && self.max_age() == other.max_age()
            && self.expires() == other.expires();

        if !so_far_so_good {
            return false;
        }

        match (self.path(), other.path()) {
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => {}
            (None, None) => {}
            _ => return false,
        };

        match (self.domain(), other.domain()) {
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => {}
            (None, None) => {}
            _ => return false,
        };

        true
    }
}

#[cfg(test)]
mod tests {
    use super::{Cookie, SameSite};
    use time::strptime;

    #[test]
    fn format() {
        let cookie = Cookie::new("foo", "bar");
        assert_eq!(&cookie.to_string(), "foo=bar");

        let cookie = Cookie::build("foo", "bar").http_only(true).finish();
        assert_eq!(&cookie.to_string(), "foo=bar; HttpOnly");

        let cookie = Cookie::build("foo", "bar").max_age(10).finish();
        assert_eq!(&cookie.to_string(), "foo=bar; Max-Age=10");

        let cookie = Cookie::build("foo", "bar").secure(true).finish();
        assert_eq!(&cookie.to_string(), "foo=bar; Secure");

        let cookie = Cookie::build("foo", "bar").path("/").finish();
        assert_eq!(&cookie.to_string(), "foo=bar; Path=/");

        let cookie = Cookie::build("foo", "bar")
            .domain("www.rust-lang.org")
            .finish();
        assert_eq!(&cookie.to_string(), "foo=bar; Domain=www.rust-lang.org");

        let time_str = "Wed, 21 Oct 2015 07:28:00 GMT";
        let expires = strptime(time_str, "%a, %d %b %Y %H:%M:%S %Z").unwrap();
        let cookie = Cookie::build("foo", "bar").expires(expires).finish();
        assert_eq!(
            &cookie.to_string(),
            "foo=bar; Expires=Wed, 21 Oct 2015 07:28:00 GMT"
        );

        let cookie = Cookie::build("foo", "bar")
            .same_site(SameSite::Strict)
            .finish();
        assert_eq!(&cookie.to_string(), "foo=bar; SameSite=Strict");

        let cookie = Cookie::build("foo", "bar")
            .same_site(SameSite::Lax)
            .finish();
        assert_eq!(&cookie.to_string(), "foo=bar; SameSite=Lax");

        let cookie = Cookie::build("foo", "bar")
            .same_site(SameSite::None)
            .finish();
        assert_eq!(&cookie.to_string(), "foo=bar; SameSite=None");
    }

    #[test]
    fn cookie_string_long_lifetimes() {
        let cookie_string =
            "bar=baz; Path=/subdir; HttpOnly; Domain=crates.io".to_owned();
        let (name, value, path, domain) = {
            // Create a cookie passing a slice
            let c = Cookie::parse(cookie_string.as_str()).unwrap();
            (c.name_raw(), c.value_raw(), c.path_raw(), c.domain_raw())
        };

        assert_eq!(name, Some("bar"));
        assert_eq!(value, Some("baz"));
        assert_eq!(path, Some("/subdir"));
        assert_eq!(domain, Some("crates.io"));
    }

    #[test]
    fn owned_cookie_string() {
        let cookie_string =
            "bar=baz; Path=/subdir; HttpOnly; Domain=crates.io".to_owned();
        let (name, value, path, domain) = {
            // Create a cookie passing an owned string
            let c = Cookie::parse(cookie_string).unwrap();
            (c.name_raw(), c.value_raw(), c.path_raw(), c.domain_raw())
        };

        assert_eq!(name, None);
        assert_eq!(value, None);
        assert_eq!(path, None);
        assert_eq!(domain, None);
    }

    #[test]
    fn owned_cookie_struct() {
        let cookie_string = "bar=baz; Path=/subdir; HttpOnly; Domain=crates.io";
        let (name, value, path, domain) = {
            // Create an owned cookie
            let c = Cookie::parse(cookie_string).unwrap().into_owned();

            (c.name_raw(), c.value_raw(), c.path_raw(), c.domain_raw())
        };

        assert_eq!(name, None);
        assert_eq!(value, None);
        assert_eq!(path, None);
        assert_eq!(domain, None);
    }

    #[test]
    fn format_encoded() {
        let cookie = Cookie::build("foo !?=", "bar;; a").finish();
        let cookie_str = cookie.encoded().to_string();
        assert_eq!(&cookie_str, "foo%20!%3F%3D=bar%3B%3B%20a");

        let cookie = Cookie::parse_encoded(cookie_str).unwrap();
        assert_eq!(cookie.name_value(), ("foo !?=", "bar;; a"));
    }
}
