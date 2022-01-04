use crate::ResourcePath;

#[allow(dead_code)]
const GEN_DELIMS: &[u8] = b":/?#[]@";

#[allow(dead_code)]
const SUB_DELIMS_WITHOUT_QS: &[u8] = b"!$'()*,";

#[allow(dead_code)]
const SUB_DELIMS: &[u8] = b"!$'()*,+?=;";

#[allow(dead_code)]
const RESERVED: &[u8] = b":/?#[]@!$'()*,+?=;";

#[allow(dead_code)]
const UNRESERVED: &[u8] = b"abcdefghijklmnopqrstuvwxyz
    ABCDEFGHIJKLMNOPQRSTUVWXYZ
    1234567890
    -._~";

const ALLOWED: &[u8] = b"abcdefghijklmnopqrstuvwxyz
    ABCDEFGHIJKLMNOPQRSTUVWXYZ
    1234567890
    -._~
    !$'()*,";

const QS: &[u8] = b"+&=;b";

thread_local! {
    static DEFAULT_QUOTER: Quoter = Quoter::new(b"@:", b"%/+");
}

#[derive(Debug, Clone, Default)]
pub struct Url {
    uri: http::Uri,
    path: Option<String>,
}

impl Url {
    #[inline]
    pub fn new(uri: http::Uri) -> Url {
        let path = DEFAULT_QUOTER.with(|q| q.requote(uri.path().as_bytes()));
        Url { uri, path }
    }

    #[inline]
    pub fn with_quoter(uri: http::Uri, quoter: &Quoter) -> Url {
        Url {
            path: quoter.requote(uri.path().as_bytes()),
            uri,
        }
    }

    #[inline]
    pub fn uri(&self) -> &http::Uri {
        &self.uri
    }

    #[inline]
    pub fn path(&self) -> &str {
        match self.path {
            Some(ref path) => path,
            _ => self.uri.path(),
        }
    }

    #[inline]
    pub fn update(&mut self, uri: &http::Uri) {
        self.uri = uri.clone();
        self.path = DEFAULT_QUOTER.with(|q| q.requote(uri.path().as_bytes()));
    }

    #[inline]
    pub fn update_with_quoter(&mut self, uri: &http::Uri, quoter: &Quoter) {
        self.uri = uri.clone();
        self.path = quoter.requote(uri.path().as_bytes());
    }
}

impl ResourcePath for Url {
    #[inline]
    fn path(&self) -> &str {
        self.path()
    }
}

/// A quoter
pub struct Quoter {
    /// Simple bit-map of safe values in the 0-127 ASCII range.
    safe_table: [u8; 16],

    /// Simple bit-map of protected values in the 0-127 ASCII range.
    protected_table: [u8; 16],
}

impl Quoter {
    pub fn new(safe: &[u8], protected: &[u8]) -> Quoter {
        let mut quoter = Quoter {
            safe_table: [0; 16],
            protected_table: [0; 16],
        };

        // prepare safe table
        for ch in 0..128 {
            if ALLOWED.contains(&ch) {
                set_bit(&mut quoter.safe_table, ch);
            }

            if QS.contains(&ch) {
                set_bit(&mut quoter.safe_table, ch);
            }
        }

        for &ch in safe {
            set_bit(&mut quoter.safe_table, ch)
        }

        // prepare protected table
        for &ch in protected {
            set_bit(&mut quoter.safe_table, ch);
            set_bit(&mut quoter.protected_table, ch);
        }

        quoter
    }

    /// Re-quotes... ?
    ///
    /// Returns `None` when no modification to the original string was required.
    pub fn requote(&self, val: &[u8]) -> Option<String> {
        let mut has_pct = 0;
        let mut pct = [b'%', 0, 0];
        let mut idx = 0;
        let mut cloned: Option<Vec<u8>> = None;

        let len = val.len();

        while idx < len {
            let ch = val[idx];

            if has_pct != 0 {
                pct[has_pct] = val[idx];
                has_pct += 1;

                if has_pct == 3 {
                    has_pct = 0;
                    let buf = cloned.as_mut().unwrap();

                    if let Some(ch) = hex_pair_to_char(pct[1], pct[2]) {
                        if ch < 128 {
                            if bit_at(&self.protected_table, ch) {
                                buf.extend_from_slice(&pct);
                                idx += 1;
                                continue;
                            }

                            if bit_at(&self.safe_table, ch) {
                                buf.push(ch);
                                idx += 1;
                                continue;
                            }
                        }

                        buf.push(ch);
                    } else {
                        buf.extend_from_slice(&pct[..]);
                    }
                }
            } else if ch == b'%' {
                has_pct = 1;

                if cloned.is_none() {
                    let mut c = Vec::with_capacity(len);
                    c.extend_from_slice(&val[..idx]);
                    cloned = Some(c);
                }
            } else if let Some(ref mut cloned) = cloned {
                cloned.push(ch)
            }

            idx += 1;
        }

        cloned.map(|data| String::from_utf8_lossy(&data).into_owned())
    }
}

/// Converts an ASCII character in the hex-encoded set (`0-9`, `A-F`, `a-f`) to its integer
/// representation from `0x0`–`0xF`.
///
/// - `0x30 ('0') => 0x0`
/// - `0x39 ('9') => 0x9`
/// - `0x41 ('a') => 0xA`
/// - `0x61 ('A') => 0xA`
/// - `0x46 ('f') => 0xF`
/// - `0x66 ('F') => 0xF`
fn from_ascii_hex(v: u8) -> Option<u8> {
    match v {
        b'0'..=b'9' => Some(v - 0x30),      // ord('0') == 0x30
        b'A'..=b'F' => Some(v - 0x41 + 10), // ord('A') == 0x41
        b'a'..=b'f' => Some(v - 0x61 + 10), // ord('a') == 0x61
        _ => None,
    }
}

/// Decode a ASCII hex-encoded pair to an integer.
///
/// Returns `None` if either portion of the decoded pair does not evaluate to a valid hex value.
///
/// - `0x33 ('3'), 0x30 ('0') => 0x30 ('0')`
/// - `0x34 ('4'), 0x31 ('1') => 0x41 ('A')`
/// - `0x36 ('6'), 0x31 ('1') => 0x61 ('a')`
fn hex_pair_to_char(d1: u8, d2: u8) -> Option<u8> {
    let (d_high, d_low) = (from_ascii_hex(d1)?, from_ascii_hex(d2)?);

    // left shift high nibble by 4 bits
    Some(d_high << 4 | d_low)
}

/// Sets bit in given bit-map to 1=true.
///
/// # Panics
/// Panics if `ch` index is out of bounds.
fn set_bit(array: &mut [u8], ch: u8) {
    array[(ch >> 3) as usize] |= 0b1 << (ch & 0b111)
}

/// Returns true if bit to true in given bit-map.
///
/// # Panics
/// Panics if `ch` index is out of bounds.
fn bit_at(array: &[u8], ch: u8) -> bool {
    array[(ch >> 3) as usize] & (0b1 << (ch & 0b111)) != 0
}

#[cfg(test)]
mod tests {
    use http::Uri;
    use std::convert::TryFrom;

    use super::*;
    use crate::{Path, ResourceDef};

    const PROTECTED: &[u8] = b"%/+";

    fn match_url(pattern: &'static str, url: impl AsRef<str>) -> Path<Url> {
        let re = ResourceDef::new(pattern);
        let uri = Uri::try_from(url.as_ref()).unwrap();
        let mut path = Path::new(Url::new(uri));
        assert!(re.capture_match_info(&mut path));
        path
    }

    fn percent_encode(data: &[u8]) -> String {
        data.iter().map(|c| format!("%{:02X}", c)).collect()
    }

    #[test]
    fn parse_url() {
        let re = "/user/{id}/test";

        let path = match_url(re, "/user/2345/test");
        assert_eq!(path.get("id").unwrap(), "2345");
    }

    #[test]
    fn protected_chars() {
        let re = "/user/{id}/test";

        let encoded = percent_encode(PROTECTED);
        let path = match_url(re, format!("/user/{}/test", encoded));
        // characters in captured segment remain unencoded
        assert_eq!(path.get("id").unwrap(), &encoded);

        // "%25" should never be decoded into '%' to guarantee the output is a valid
        // percent-encoded format
        let path = match_url(re, "/user/qwe%25/test");
        assert_eq!(path.get("id").unwrap(), "qwe%25");

        let path = match_url(re, "/user/qwe%25rty/test");
        assert_eq!(path.get("id").unwrap(), "qwe%25rty");
    }

    #[test]
    fn non_protected_ascii() {
        let non_protected_ascii = ('\u{0}'..='\u{7F}')
            .filter(|&c| c.is_ascii() && !PROTECTED.contains(&(c as u8)))
            .collect::<String>();
        let encoded = percent_encode(non_protected_ascii.as_bytes());
        let path = match_url("/user/{id}/test", format!("/user/{}/test", encoded));
        assert_eq!(path.get("id").unwrap(), &non_protected_ascii);
    }

    #[test]
    fn valid_utf8_multibyte() {
        let test = ('\u{FF00}'..='\u{FFFF}').collect::<String>();
        let encoded = percent_encode(test.as_bytes());
        let path = match_url("/a/{id}/b", format!("/a/{}/b", &encoded));
        assert_eq!(path.get("id").unwrap(), &test);
    }

    #[test]
    fn invalid_utf8() {
        let invalid_utf8 = percent_encode((0x80..=0xff).collect::<Vec<_>>().as_slice());
        let uri = Uri::try_from(format!("/{}", invalid_utf8)).unwrap();
        let path = Path::new(Url::new(uri));

        // We should always get a valid utf8 string
        assert!(String::from_utf8(path.path().as_bytes().to_owned()).is_ok());
    }

    #[test]
    fn hex_encoding() {
        let hex = b"0123456789abcdefABCDEF";

        for i in 0..256 {
            let c = i as u8;
            if hex.contains(&c) {
                assert!(from_ascii_hex(c).is_some())
            } else {
                assert!(from_ascii_hex(c).is_none())
            }
        }

        let expected = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 10, 11, 12, 13, 14, 15,
        ];
        for i in 0..hex.len() {
            assert_eq!(from_ascii_hex(hex[i]).unwrap(), expected[i]);
        }
    }

    #[test]
    fn custom_quoter() {
        let q = Quoter::new(b"", b"+");
        assert_eq!(q.requote(b"/a%25c").unwrap(), "/a%c");
        assert_eq!(q.requote(b"/a%2Bc").unwrap(), "/a%2Bc");

        let q = Quoter::new(b"%+", b"/");
        assert_eq!(q.requote(b"/a%25b%2Bc").unwrap(), "/a%b+c");
        assert_eq!(q.requote(b"/a%2fb").unwrap(), "/a%2fb");
        assert_eq!(q.requote(b"/a%2Fb").unwrap(), "/a%2Fb");
    }

    #[test]
    fn quoter_no_modification() {
        let q = Quoter::new(b"", b"");
        assert_eq!(q.requote(b"/abc/../efg"), None);
    }
}
