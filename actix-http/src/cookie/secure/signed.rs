use ring::hmac::{self, sign, verify, Algorithm, HMAC_SHA256};

use super::Key;
use crate::cookie::{Cookie, CookieJar};

// Keep these in sync, and keep the key len synced with the `signed` docs as
// well as the `KEYS_INFO` const in secure::Key.
static HMAC_DIGEST: Algorithm = HMAC_SHA256;
const BASE64_DIGEST_LEN: usize = 44;
pub const KEY_LEN: usize = 32;

/// A child cookie jar that authenticates its cookies.
///
/// A _signed_ child jar signs all the cookies added to it and verifies cookies
/// retrieved from it. Any cookies stored in a `SignedJar` are assured integrity
/// and authenticity. In other words, clients cannot tamper with the contents of
/// a cookie nor can they fabricate cookie values, but the data is visible in
/// plaintext.
///
/// This type is only available when the `secure` feature is enabled.
pub struct SignedJar<'a> {
    parent: &'a mut CookieJar,
    key: hmac::Key,
}

impl<'a> SignedJar<'a> {
    /// Creates a new child `SignedJar` with parent `parent` and key `key`. This
    /// method is typically called indirectly via the `signed` method of
    /// `CookieJar`.
    #[doc(hidden)]
    pub fn new(parent: &'a mut CookieJar, key: &Key) -> SignedJar<'a> {
        SignedJar {
            parent: parent,
            key: hmac::Key::new(HMAC_DIGEST, key.signing()),
        }
    }

    /// Given a signed value `str` where the signature is prepended to `value`,
    /// verifies the signed value and returns it. If there's a problem, returns
    /// an `Err` with a string describing the issue.
    fn verify(&self, cookie_value: &str) -> Result<String, &'static str> {
        if cookie_value.len() < BASE64_DIGEST_LEN {
            return Err("length of value is <= BASE64_DIGEST_LEN");
        }

        let (digest_str, value) = cookie_value.split_at(BASE64_DIGEST_LEN);
        let sig = base64::decode(digest_str).map_err(|_| "bad base64 digest")?;

        verify(&self.key, value.as_bytes(), &sig)
            .map(|_| value.to_string())
            .map_err(|_| "value did not verify")
    }

    /// Returns a reference to the `Cookie` inside this jar with the name `name`
    /// and verifies the authenticity and integrity of the cookie's value,
    /// returning a `Cookie` with the authenticated value. If the cookie cannot
    /// be found, or the cookie fails to verify, `None` is returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::{CookieJar, Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut jar = CookieJar::new();
    /// let mut signed_jar = jar.signed(&key);
    /// assert!(signed_jar.get("name").is_none());
    ///
    /// signed_jar.add(Cookie::new("name", "value"));
    /// assert_eq!(signed_jar.get("name").unwrap().value(), "value");
    /// ```
    pub fn get(&self, name: &str) -> Option<Cookie<'static>> {
        if let Some(cookie_ref) = self.parent.get(name) {
            let mut cookie = cookie_ref.clone();
            if let Ok(value) = self.verify(cookie.value()) {
                cookie.set_value(value);
                return Some(cookie);
            }
        }

        None
    }

    /// Adds `cookie` to the parent jar. The cookie's value is signed assuring
    /// integrity and authenticity.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::{CookieJar, Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut jar = CookieJar::new();
    /// jar.signed(&key).add(Cookie::new("name", "value"));
    ///
    /// assert_ne!(jar.get("name").unwrap().value(), "value");
    /// assert!(jar.get("name").unwrap().value().contains("value"));
    /// assert_eq!(jar.signed(&key).get("name").unwrap().value(), "value");
    /// ```
    pub fn add(&mut self, mut cookie: Cookie<'static>) {
        self.sign_cookie(&mut cookie);
        self.parent.add(cookie);
    }

    /// Adds an "original" `cookie` to this jar. The cookie's value is signed
    /// assuring integrity and authenticity. Adding an original cookie does not
    /// affect the [`CookieJar::delta()`](struct.CookieJar.html#method.delta)
    /// computation. This method is intended to be used to seed the cookie jar
    /// with cookies received from a client's HTTP message.
    ///
    /// For accurate `delta` computations, this method should not be called
    /// after calling `remove`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::{CookieJar, Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut jar = CookieJar::new();
    /// jar.signed(&key).add_original(Cookie::new("name", "value"));
    ///
    /// assert_eq!(jar.iter().count(), 1);
    /// assert_eq!(jar.delta().count(), 0);
    /// ```
    pub fn add_original(&mut self, mut cookie: Cookie<'static>) {
        self.sign_cookie(&mut cookie);
        self.parent.add_original(cookie);
    }

    /// Signs the cookie's value assuring integrity and authenticity.
    fn sign_cookie(&self, cookie: &mut Cookie) {
        let digest = sign(&self.key, cookie.value().as_bytes());
        let mut new_value = base64::encode(digest.as_ref());
        new_value.push_str(cookie.value());
        cookie.set_value(new_value);
    }

    /// Removes `cookie` from the parent jar.
    ///
    /// For correct removal, the passed in `cookie` must contain the same `path`
    /// and `domain` as the cookie that was initially set.
    ///
    /// See [CookieJar::remove](struct.CookieJar.html#method.remove) for more
    /// details.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::{CookieJar, Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut jar = CookieJar::new();
    /// let mut signed_jar = jar.signed(&key);
    ///
    /// signed_jar.add(Cookie::new("name", "value"));
    /// assert!(signed_jar.get("name").is_some());
    ///
    /// signed_jar.remove(Cookie::named("name"));
    /// assert!(signed_jar.get("name").is_none());
    /// ```
    pub fn remove(&mut self, cookie: Cookie<'static>) {
        self.parent.remove(cookie);
    }
}

#[cfg(test)]
mod test {
    use super::{Cookie, CookieJar, Key};

    #[test]
    fn simple() {
        let key = Key::generate();
        let mut jar = CookieJar::new();
        assert_simple_behaviour!(jar, jar.signed(&key));
    }

    #[test]
    fn private() {
        let key = Key::generate();
        let mut jar = CookieJar::new();
        assert_secure_behaviour!(jar, jar.signed(&key));
    }
}
