use ring::digest::{Algorithm, SHA256};
use ring::hkdf::expand;
use ring::hmac::SigningKey;
use ring::rand::{SecureRandom, SystemRandom};

use super::private::KEY_LEN as PRIVATE_KEY_LEN;
use super::signed::KEY_LEN as SIGNED_KEY_LEN;

static HKDF_DIGEST: &Algorithm = &SHA256;
const KEYS_INFO: &str = "COOKIE;SIGNED:HMAC-SHA256;PRIVATE:AEAD-AES-256-GCM";

/// A cryptographic master key for use with `Signed` and/or `Private` jars.
///
/// This structure encapsulates secure, cryptographic keys for use with both
/// [PrivateJar](struct.PrivateJar.html) and [SignedJar](struct.SignedJar.html).
/// It can be derived from a single master key via
/// [from_master](#method.from_master) or generated from a secure random source
/// via [generate](#method.generate). A single instance of `Key` can be used for
/// both a `PrivateJar` and a `SignedJar`.
///
/// This type is only available when the `secure` feature is enabled.
#[derive(Clone)]
pub struct Key {
    signing_key: [u8; SIGNED_KEY_LEN],
    encryption_key: [u8; PRIVATE_KEY_LEN],
}

impl Key {
    /// Derives new signing/encryption keys from a master key.
    ///
    /// The master key must be at least 256-bits (32 bytes). For security, the
    /// master key _must_ be cryptographically random. The keys are derived
    /// deterministically from the master key.
    ///
    /// # Panics
    ///
    /// Panics if `key` is less than 32 bytes in length.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Key;
    ///
    /// # /*
    /// let master_key = { /* a cryptographically random key >= 32 bytes */ };
    /// # */
    /// # let master_key: &Vec<u8> = &(0..32).collect();
    ///
    /// let key = Key::from_master(master_key);
    /// ```
    pub fn from_master(key: &[u8]) -> Key {
        if key.len() < 32 {
            panic!(
                "bad master key length: expected at least 32 bytes, found {}",
                key.len()
            );
        }

        // Expand the user's key into two.
        let prk = SigningKey::new(HKDF_DIGEST, key);
        let mut both_keys = [0; SIGNED_KEY_LEN + PRIVATE_KEY_LEN];
        expand(&prk, KEYS_INFO.as_bytes(), &mut both_keys);

        // Copy the keys into their respective arrays.
        let mut signing_key = [0; SIGNED_KEY_LEN];
        let mut encryption_key = [0; PRIVATE_KEY_LEN];
        signing_key.copy_from_slice(&both_keys[..SIGNED_KEY_LEN]);
        encryption_key.copy_from_slice(&both_keys[SIGNED_KEY_LEN..]);

        Key {
            signing_key,
            encryption_key,
        }
    }

    /// Generates signing/encryption keys from a secure, random source. Keys are
    /// generated nondeterministically.
    ///
    /// # Panics
    ///
    /// Panics if randomness cannot be retrieved from the operating system. See
    /// [try_generate](#method.try_generate) for a non-panicking version.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Key;
    ///
    /// let key = Key::generate();
    /// ```
    pub fn generate() -> Key {
        Self::try_generate().expect("failed to generate `Key` from randomness")
    }

    /// Attempts to generate signing/encryption keys from a secure, random
    /// source. Keys are generated nondeterministically. If randomness cannot be
    /// retrieved from the underlying operating system, returns `None`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Key;
    ///
    /// let key = Key::try_generate();
    /// ```
    pub fn try_generate() -> Option<Key> {
        let mut sign_key = [0; SIGNED_KEY_LEN];
        let mut enc_key = [0; PRIVATE_KEY_LEN];

        let rng = SystemRandom::new();
        if rng.fill(&mut sign_key).is_err() || rng.fill(&mut enc_key).is_err() {
            return None;
        }

        Some(Key {
            signing_key: sign_key,
            encryption_key: enc_key,
        })
    }

    /// Returns the raw bytes of a key suitable for signing cookies.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Key;
    ///
    /// let key = Key::generate();
    /// let signing_key = key.signing();
    /// ```
    pub fn signing(&self) -> &[u8] {
        &self.signing_key[..]
    }

    /// Returns the raw bytes of a key suitable for encrypting cookies.
    ///
    /// # Example
    ///
    /// ```rust
    /// use actix_http::cookie::Key;
    ///
    /// let key = Key::generate();
    /// let encryption_key = key.encryption();
    /// ```
    pub fn encryption(&self) -> &[u8] {
        &self.encryption_key[..]
    }
}

#[cfg(test)]
mod test {
    use super::Key;

    #[test]
    fn deterministic_from_master() {
        let master_key: Vec<u8> = (0..32).collect();

        let key_a = Key::from_master(&master_key);
        let key_b = Key::from_master(&master_key);

        assert_eq!(key_a.signing(), key_b.signing());
        assert_eq!(key_a.encryption(), key_b.encryption());
        assert_ne!(key_a.encryption(), key_a.signing());

        let master_key_2: Vec<u8> = (32..64).collect();
        let key_2 = Key::from_master(&master_key_2);

        assert_ne!(key_2.signing(), key_a.signing());
        assert_ne!(key_2.encryption(), key_a.encryption());
    }

    #[test]
    fn non_deterministic_generate() {
        let key_a = Key::generate();
        let key_b = Key::generate();

        assert_ne!(key_a.signing(), key_b.signing());
        assert_ne!(key_a.encryption(), key_b.encryption());
    }
}
