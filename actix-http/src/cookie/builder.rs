use std::borrow::Cow;

use time::{Duration, OffsetDateTime};

use super::{Cookie, SameSite};

/// Structure that follows the builder pattern for building `Cookie` structs.
///
/// To construct a cookie:
///
///   1. Call [`Cookie::build`](struct.Cookie.html#method.build) to start building.
///   2. Use any of the builder methods to set fields in the cookie.
///   3. Call [finish](#method.finish) to retrieve the built cookie.
///
/// # Example
///
/// ```rust
/// use actix_http::cookie::Cookie;
///
/// let cookie: Cookie = Cookie::build("name", "value")
///     .domain("www.rust-lang.org")
///     .path("/")
///     .secure(true)
///     .http_only(true)
///     .max_age(84600)
///     .finish();
/// ```
#[derive(Debug, Clone)]
pub struct CookieBuilder {
    /// The cookie being built.
    cookie: Cookie<'static>,
}

impl CookieBuilder {
    /// Creates a new `CookieBuilder` instance from the given name and value.
    ///
    /// This method is typically called indirectly via
    /// [Cookie::build](struct.Cookie.html#method.build).
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::build("foo", "bar").finish();
    /// assert_eq!(c.name_value(), ("foo", "bar"));
    /// ```
    pub fn new<N, V>(name: N, value: V) -> CookieBuilder
    where
        N: Into<Cow<'static, str>>,
        V: Into<Cow<'static, str>>,
    {
        CookieBuilder {
            cookie: Cookie::new(name, value),
        }
    }

    /// Sets the `expires` field in the cookie being built.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::build("foo", "bar")
    ///     .expires(time::OffsetDateTime::now())
    ///     .finish();
    ///
    /// assert!(c.expires().is_some());
    /// ```
    #[inline]
    pub fn expires(mut self, when: OffsetDateTime) -> CookieBuilder {
        self.cookie.set_expires(when);
        self
    }

    /// Sets the `max_age` field in seconds in the cookie being built.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::build("foo", "bar")
    ///     .max_age(1800)
    ///     .finish();
    ///
    /// assert_eq!(c.max_age(), Some(time::Duration::seconds(30 * 60)));
    /// ```
    #[inline]
    pub fn max_age(self, seconds: i64) -> CookieBuilder {
        self.max_age_time(Duration::seconds(seconds))
    }

    /// Sets the `max_age` field in the cookie being built.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::build("foo", "bar")
    ///     .max_age_time(time::Duration::minutes(30))
    ///     .finish();
    ///
    /// assert_eq!(c.max_age(), Some(time::Duration::seconds(30 * 60)));
    /// ```
    #[inline]
    pub fn max_age_time(mut self, value: Duration) -> CookieBuilder {
        // Truncate any nanoseconds from the Duration, as they aren't represented within `Max-Age`
        // and would cause two otherwise identical `Cookie` instances to not be equivalent to one another.
        self.cookie
            .set_max_age(Duration::seconds(value.whole_seconds()));
        self
    }

    /// Sets the `domain` field in the cookie being built.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::build("foo", "bar")
    ///     .domain("www.rust-lang.org")
    ///     .finish();
    ///
    /// assert_eq!(c.domain(), Some("www.rust-lang.org"));
    /// ```
    pub fn domain<D: Into<Cow<'static, str>>>(mut self, value: D) -> CookieBuilder {
        self.cookie.set_domain(value);
        self
    }

    /// Sets the `path` field in the cookie being built.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::build("foo", "bar")
    ///     .path("/")
    ///     .finish();
    ///
    /// assert_eq!(c.path(), Some("/"));
    /// ```
    pub fn path<P: Into<Cow<'static, str>>>(mut self, path: P) -> CookieBuilder {
        self.cookie.set_path(path);
        self
    }

    /// Sets the `secure` field in the cookie being built.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::build("foo", "bar")
    ///     .secure(true)
    ///     .finish();
    ///
    /// assert_eq!(c.secure(), Some(true));
    /// ```
    #[inline]
    pub fn secure(mut self, value: bool) -> CookieBuilder {
        self.cookie.set_secure(value);
        self
    }

    /// Sets the `http_only` field in the cookie being built.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::build("foo", "bar")
    ///     .http_only(true)
    ///     .finish();
    ///
    /// assert_eq!(c.http_only(), Some(true));
    /// ```
    #[inline]
    pub fn http_only(mut self, value: bool) -> CookieBuilder {
        self.cookie.set_http_only(value);
        self
    }

    /// Sets the `same_site` field in the cookie being built.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::{Cookie, SameSite};
    ///
    /// let c = Cookie::build("foo", "bar")
    ///     .same_site(SameSite::Strict)
    ///     .finish();
    ///
    /// assert_eq!(c.same_site(), Some(SameSite::Strict));
    /// ```
    #[inline]
    pub fn same_site(mut self, value: SameSite) -> CookieBuilder {
        self.cookie.set_same_site(value);
        self
    }

    /// Makes the cookie being built 'permanent' by extending its expiration and
    /// max age 20 years into the future.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    /// use time::Duration;
    ///
    /// let c = Cookie::build("foo", "bar")
    ///     .permanent()
    ///     .finish();
    ///
    /// assert_eq!(c.max_age(), Some(Duration::days(365 * 20)));
    /// # assert!(c.expires().is_some());
    /// ```
    #[inline]
    pub fn permanent(mut self) -> CookieBuilder {
        self.cookie.make_permanent();
        self
    }

    /// Finishes building and returns the built `Cookie`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Cookie;
    ///
    /// let c = Cookie::build("foo", "bar")
    ///     .domain("crates.io")
    ///     .path("/")
    ///     .finish();
    ///
    /// assert_eq!(c.name_value(), ("foo", "bar"));
    /// assert_eq!(c.domain(), Some("crates.io"));
    /// assert_eq!(c.path(), Some("/"));
    /// ```
    #[inline]
    pub fn finish(self) -> Cookie<'static> {
        self.cookie
    }
}
