use std::str;

use log::warn;
use ring::aead::{open_in_place, seal_in_place, Aad, Algorithm, Nonce, AES_256_GCM};
use ring::aead::{OpeningKey, SealingKey};
use ring::rand::{SecureRandom, SystemRandom};

use super::Key;
use crate::cookie::{Cookie, CookieJar};

// Keep these in sync, and keep the key len synced with the `private` docs as
// well as the `KEYS_INFO` const in secure::Key.
static ALGO: &Algorithm = &AES_256_GCM;
const NONCE_LEN: usize = 12;
pub const KEY_LEN: usize = 32;

/// A child cookie jar that provides authenticated encryption for its cookies.
///
/// A _private_ child jar signs and encrypts all the cookies added to it and
/// verifies and decrypts cookies retrieved from it. Any cookies stored in a
/// `PrivateJar` are simultaneously assured confidentiality, integrity, and
/// authenticity. In other words, clients cannot discover nor tamper with the
/// contents of a cookie, nor can they fabricate cookie data.
///
/// This type is only available when the `secure` feature is enabled.
pub struct PrivateJar<'a> {
    parent: &'a mut CookieJar,
    key: [u8; KEY_LEN],
}

impl<'a> PrivateJar<'a> {
    /// Creates a new child `PrivateJar` with parent `parent` and key `key`.
    /// This method is typically called indirectly via the `signed` method of
    /// `CookieJar`.
    #[doc(hidden)]
    pub fn new(parent: &'a mut CookieJar, key: &Key) -> PrivateJar<'a> {
        let mut key_array = [0u8; KEY_LEN];
        key_array.copy_from_slice(key.encryption());
        PrivateJar {
            parent,
            key: key_array,
        }
    }

    /// Given a sealed value `str` and a key name `name`, where the nonce is
    /// prepended to the original value and then both are Base64 encoded,
    /// verifies and decrypts the sealed value and returns it. If there's a
    /// problem, returns an `Err` with a string describing the issue.
    fn unseal(&self, name: &str, value: &str) -> Result<String, &'static str> {
        let mut data = base64::decode(value).map_err(|_| "bad base64 value")?;
        if data.len() <= NONCE_LEN {
            return Err("length of decoded data is <= NONCE_LEN");
        }

        let ad = Aad::from(name.as_bytes());
        let key = OpeningKey::new(ALGO, &self.key).expect("opening key");
        let (nonce, sealed) = data.split_at_mut(NONCE_LEN);
        let nonce =
            Nonce::try_assume_unique_for_key(nonce).expect("invalid length of `nonce`");
        let unsealed = open_in_place(&key, nonce, ad, 0, sealed)
            .map_err(|_| "invalid key/nonce/value: bad seal")?;

        if let Ok(unsealed_utf8) = str::from_utf8(unsealed) {
            Ok(unsealed_utf8.to_string())
        } else {
            warn!(
                "Private cookie does not have utf8 content!
It is likely the secret key used to encrypt them has been leaked.
Please change it as soon as possible."
            );
            Err("bad unsealed utf8")
        }
    }

    /// Returns a reference to the `Cookie` inside this jar with the name `name`
    /// and authenticates and decrypts the cookie's value, returning a `Cookie`
    /// with the decrypted value. If the cookie cannot be found, or the cookie
    /// fails to authenticate or decrypt, `None` is returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::{CookieJar, Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut jar = CookieJar::new();
    /// let mut private_jar = jar.private(&key);
    /// assert!(private_jar.get("name").is_none());
    ///
    /// private_jar.add(Cookie::new("name", "value"));
    /// assert_eq!(private_jar.get("name").unwrap().value(), "value");
    /// ```
    pub fn get(&self, name: &str) -> Option<Cookie<'static>> {
        if let Some(cookie_ref) = self.parent.get(name) {
            let mut cookie = cookie_ref.clone();
            if let Ok(value) = self.unseal(name, cookie.value()) {
                cookie.set_value(value);
                return Some(cookie);
            }
        }

        None
    }

    /// Adds `cookie` to the parent jar. The cookie's value is encrypted with
    /// authenticated encryption assuring confidentiality, integrity, and
    /// authenticity.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::{CookieJar, Cookie, Key};
    ///
    /// let key = Key::generate();
    /// let mut jar = CookieJar::new();
    /// jar.private(&key).add(Cookie::new("name", "value"));
    ///
    /// assert_ne!(jar.get("name").unwrap().value(), "value");
    /// assert_eq!(jar.private(&key).get("name").unwrap().value(), "value");
    /// ```
    pub fn add(&mut self, mut cookie: Cookie<'static>) {
        self.encrypt_cookie(&mut cookie);

        // Add the sealed cookie to the parent.
        self.parent.add(cookie);
    }

    /// Adds an "original" `cookie` to parent jar. The cookie's value is
    /// encrypted with authenticated encryption assuring confidentiality,
    /// integrity, and authenticity. Adding an original cookie does not affect
    /// the [`CookieJar::delta()`](struct.CookieJar.html#method.delta)
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
    /// jar.private(&key).add_original(Cookie::new("name", "value"));
    ///
    /// assert_eq!(jar.iter().count(), 1);
    /// assert_eq!(jar.delta().count(), 0);
    /// ```
    pub fn add_original(&mut self, mut cookie: Cookie<'static>) {
        self.encrypt_cookie(&mut cookie);

        // Add the sealed cookie to the parent.
        self.parent.add_original(cookie);
    }

    /// Encrypts the cookie's value with
    /// authenticated encryption assuring confidentiality, integrity, and authenticity.
    fn encrypt_cookie(&self, cookie: &mut Cookie) {
        let name = cookie.name().as_bytes();
        let value = cookie.value().as_bytes();
        let data = encrypt_name_value(name, value, &self.key);

        // Base64 encode the nonce and encrypted value.
        let sealed_value = base64::encode(&data);
        cookie.set_value(sealed_value);
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
    /// let mut private_jar = jar.private(&key);
    ///
    /// private_jar.add(Cookie::new("name", "value"));
    /// assert!(private_jar.get("name").is_some());
    ///
    /// private_jar.remove(Cookie::named("name"));
    /// assert!(private_jar.get("name").is_none());
    /// ```
    pub fn remove(&mut self, cookie: Cookie<'static>) {
        self.parent.remove(cookie);
    }
}

fn encrypt_name_value(name: &[u8], value: &[u8], key: &[u8]) -> Vec<u8> {
    // Create the `SealingKey` structure.
    let key = SealingKey::new(ALGO, key).expect("sealing key creation");

    // Create a vec to hold the [nonce | cookie value | overhead].
    let overhead = ALGO.tag_len();
    let mut data = vec![0; NONCE_LEN + value.len() + overhead];

    // Randomly generate the nonce, then copy the cookie value as input.
    let (nonce, in_out) = data.split_at_mut(NONCE_LEN);
    SystemRandom::new()
        .fill(nonce)
        .expect("couldn't random fill nonce");
    in_out[..value.len()].copy_from_slice(value);
    let nonce =
        Nonce::try_assume_unique_for_key(nonce).expect("invalid length of `nonce`");

    // Use cookie's name as associated data to prevent value swapping.
    let ad = Aad::from(name);

    // Perform the actual sealing operation and get the output length.
    let output_len =
        seal_in_place(&key, nonce, ad, in_out, overhead).expect("in-place seal");

    // Remove the overhead and return the sealed content.
    data.truncate(NONCE_LEN + output_len);
    data
}

#[cfg(test)]
mod test {
    use super::{encrypt_name_value, Cookie, CookieJar, Key};

    #[test]
    fn simple() {
        let key = Key::generate();
        let mut jar = CookieJar::new();
        assert_simple_behaviour!(jar, jar.private(&key));
    }

    #[test]
    fn private() {
        let key = Key::generate();
        let mut jar = CookieJar::new();
        assert_secure_behaviour!(jar, jar.private(&key));
    }

    #[test]
    fn non_utf8() {
        let key = Key::generate();
        let mut jar = CookieJar::new();

        let name = "malicious";
        let mut assert_non_utf8 = |value: &[u8]| {
            let sealed = encrypt_name_value(name.as_bytes(), value, &key.encryption());
            let encoded = base64::encode(&sealed);
            assert_eq!(
                jar.private(&key).unseal(name, &encoded),
                Err("bad unsealed utf8")
            );
            jar.add(Cookie::new(name, encoded));
            assert_eq!(jar.private(&key).get(name), None);
        };

        assert_non_utf8(&[0x72, 0xfb, 0xdf, 0x74]); // rûst in ISO/IEC 8859-1

        let mut malicious =
            String::from(r#"{"id":"abc123??%X","admin":true}"#).into_bytes();
        malicious[8] |= 0b1100_0000;
        malicious[9] |= 0b1100_0000;
        assert_non_utf8(&malicious);
    }
}
