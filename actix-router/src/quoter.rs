#[allow(dead_code)]
mod charsets {
    pub const GEN_DELIMS: &[u8] = b":/?#[]@";
    pub const SUB_DELIMS_WITHOUT_QS: &[u8] = b"!$'()*,";
    pub const SUB_DELIMS: &[u8] = b"!$'()*,+?=;";
    pub const RESERVED: &[u8] = b":/?#[]@!$'()*,+?=;";
    pub const UNRESERVED: &[u8] = b"abcdefghijklmnopqrstuvwxyz
    ABCDEFGHIJKLMNOPQRSTUVWXYZ
    1234567890
    -._~";
    pub const ALLOWED: &[u8] = b"abcdefghijklmnopqrstuvwxyz
    ABCDEFGHIJKLMNOPQRSTUVWXYZ
    1234567890
    -._~
    !$'()*,";
    pub const QS: &[u8] = b"+&=;b";
}

/// Partial Percent-decoding.
pub struct Quoter {
    /// Simple bit-map of protected values in the 0-127 ASCII range.
    protected_table: [u8; 16],
}

impl Quoter {
    /// Constructs a new `Quoter` instance given a set of protected ASCII bytes.
    ///
    /// The first argument is ignored and is kept for backward compatibility only.
    ///
    /// # Panic
    /// Panics if any of the `protected` bytes is not in the 0-127 ASCII range.
    pub fn new(_: &[u8], protected: &[u8]) -> Quoter {
        let mut protected_table = [0; 16];

        // prepare protected table
        for &ch in protected {
            set_bit(&mut protected_table, ch);
        }

        Quoter { protected_table }
    }

    /// Decodes the next escape sequence, if any, and advances `val`.
    #[inline(always)]
    fn decode_next<'a>(&self, val: &mut &'a [u8]) -> Option<(&'a [u8], u8)> {
        for i in 0..val.len() {
            if let (prev, [b'%', p1, p2, rem @ ..]) = val.split_at(i) {
                if let Some(ch) = hex_pair_to_char(*p1, *p2)
                    // ingore protected ascii bytes
                    .filter(|&ch| !(ch < 128 && bit_at(&self.protected_table, ch)))
                {
                    *val = rem;
                    return Some((prev, ch));
                }
            }
        }

        None
    }

    /// Partially percent-decodes the given bytes.
    /// Escape sequences of the protected set are *not* decoded.
    ///
    /// Returns `None` when no modification to the original bytes was required.
    ///
    /// Invalid/incomplete percent-encoding sequences are passed unmodified.
    pub fn requote(&self, val: &[u8]) -> Option<Vec<u8>> {
        let mut remaining = val;

        let (prev, ch) = self.decode_next(&mut remaining)?;
        let mut buf = Vec::<u8>::with_capacity(val.len());
        buf.extend_from_slice(prev);
        buf.push(ch);

        while let Some((prev, ch)) = self.decode_next(&mut remaining) {
            // this ugly conditional achieves +50% perf in cases where this is a tight loop.
            if prev.len() != 0 {
                buf.extend_from_slice(prev);
            }
            buf.push(ch);
        }

        buf.extend_from_slice(remaining);
        Some(buf)
    }

    pub(crate) fn requote_str_lossy(&self, val: &str) -> Option<String> {
        self.requote(val.as_bytes())
            .map(|data| String::from_utf8_lossy(&data).into_owned())
    }
}

/// Decode a ASCII hex-encoded pair to an integer.
///
/// Returns `None` if either portion of the decoded pair does not evaluate to a valid hex value.
///
/// - `0x33 ('3'), 0x30 ('0') => 0x30 ('0')`
/// - `0x34 ('4'), 0x31 ('1') => 0x41 ('A')`
/// - `0x36 ('6'), 0x31 ('1') => 0x61 ('a')`
#[inline(always)]
fn hex_pair_to_char(d1: u8, d2: u8) -> Option<u8> {
    let d_high = char::from(d1).to_digit(16)?;
    let d_low = char::from(d2).to_digit(16)?;

    // left shift high nibble by 4 bits
    Some((d_high as u8) << 4 | (d_low as u8))
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
fn bit_at(array: &[u8; 16], ch: u8) -> bool {
    array[(ch >> 3) as usize] & (0b1 << (ch & 0b111)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_quoter() {
        let q = Quoter::new(b"", b"+");
        assert_eq!(q.requote(b"/a%25c").unwrap(), b"/a%c");
        assert_eq!(q.requote(b"/a%2Bc"), None);

        let q = Quoter::new(b"%+", b"/");
        assert_eq!(q.requote(b"/a%25b%2Bc").unwrap(), b"/a%b+c");
        assert_eq!(q.requote(b"/a%2fb"), None);
        assert_eq!(q.requote(b"/a%2Fb"), None);
        assert_eq!(q.requote(b"/a%0Ab").unwrap(), b"/a\nb");
        assert_eq!(q.requote(b"/a%FE\xffb").unwrap(), b"/a\xfe\xffb");
        assert_eq!(q.requote(b"/a\xfe\xffb"), None);
    }

    #[test]
    fn non_ascii() {
        let q = Quoter::new(b"%+", b"/");
        assert_eq!(q.requote(b"/a%FE\xffb").unwrap(), b"/a\xfe\xffb");
        assert_eq!(q.requote(b"/a\xfe\xffb"), None);
    }

    #[test]
    fn invalid_sequences() {
        let q = Quoter::new(b"%+", b"/");
        assert_eq!(q.requote(b"/a%2x%2X%%"), None);
        assert_eq!(q.requote(b"/a%20%2X%%").unwrap(), b"/a %2X%%");
    }

    #[test]
    fn quoter_no_modification() {
        let q = Quoter::new(b"", b"");
        assert_eq!(q.requote(b"/abc/../efg"), None);
    }
}
