use http::Uri;
use std::rc::Rc;

// https://tools.ietf.org/html/rfc3986#section-2.2
const RESERVED_PLUS_EXTRA: &[u8] = b":/?#[]@!$&'()*,+?;=%^ <>\"\\`{}|";

// https://tools.ietf.org/html/rfc3986#section-2.3
const UNRESERVED: &[u8] =
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890-._~";

#[inline]
fn bit_at(array: &[u8], ch: u8) -> bool {
    array[(ch >> 3) as usize] & (1 << (ch & 7)) != 0
}

#[inline]
fn set_bit(array: &mut [u8], ch: u8) {
    array[(ch >> 3) as usize] |= 1 << (ch & 7)
}

lazy_static! {
    static ref UNRESERVED_QUOTER: Quoter = { Quoter::new(UNRESERVED) };
    pub(crate) static ref RESERVED_QUOTER: Quoter = { Quoter::new(RESERVED_PLUS_EXTRA) };
}

#[derive(Default, Clone, Debug)]
pub(crate) struct Url {
    uri: Uri,
    path: Option<Rc<String>>,
}

impl Url {
    pub fn new(uri: Uri) -> Url {
        let path = UNRESERVED_QUOTER.requote(uri.path().as_bytes());

        Url { uri, path }
    }

    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    pub fn path(&self) -> &str {
        if let Some(ref s) = self.path {
            s
        } else {
            self.uri.path()
        }
    }
}

pub(crate) struct Quoter {
    safe_table: [u8; 16],
}

impl Quoter {
    pub fn new(safe: &[u8]) -> Quoter {
        let mut q = Quoter {
            safe_table: [0; 16],
        };

        // prepare safe table
        for ch in safe {
            set_bit(&mut q.safe_table, *ch)
        }

        q
    }

    pub fn requote(&self, val: &[u8]) -> Option<Rc<String>> {
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

                    if let Some(ch) = restore_ch(pct[1], pct[2]) {
                        if ch < 128 {
                            if bit_at(&self.safe_table, ch) {
                                buf.push(ch);
                                idx += 1;
                                continue;
                            }

                            buf.extend_from_slice(&pct);
                        } else {
                            // Not ASCII, decode it
                            buf.push(ch);
                        }
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

        if let Some(data) = cloned {
            // Unsafe: we get data from http::Uri, which does utf-8 checks already
            // this code only decodes valid pct encoded values
            Some(Rc::new(unsafe { String::from_utf8_unchecked(data) }))
        } else {
            None
        }
    }
}

#[inline]
fn from_hex(v: u8) -> Option<u8> {
    if v >= b'0' && v <= b'9' {
        Some(v - 0x30) // ord('0') == 0x30
    } else if v >= b'A' && v <= b'F' {
        Some(v - 0x41 + 10) // ord('A') == 0x41
    } else if v > b'a' && v <= b'f' {
        Some(v - 0x61 + 10) // ord('a') == 0x61
    } else {
        None
    }
}

#[inline]
fn restore_ch(d1: u8, d2: u8) -> Option<u8> {
    from_hex(d1).and_then(|d1| from_hex(d2).and_then(move |d2| Some(d1 << 4 | d2)))
}


#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    #[test]
    fn decode_path() {
        assert_eq!(UNRESERVED_QUOTER.requote(b"https://localhost:80/foo"), None);

        assert_eq!(
            Rc::try_unwrap(UNRESERVED_QUOTER.requote(
                b"https://localhost:80/foo%25"
            ).unwrap()).unwrap(),
            "https://localhost:80/foo%25".to_string()
        );

        assert_eq!(
            Rc::try_unwrap(UNRESERVED_QUOTER.requote(
                b"http://cache-service/http%3A%2F%2Flocalhost%3A80%2Ffoo"
            ).unwrap()).unwrap(),
            "http://cache-service/http%3A%2F%2Flocalhost%3A80%2Ffoo".to_string()
        );

        assert_eq!(
            Rc::try_unwrap(UNRESERVED_QUOTER.requote(
                b"http://cache/http%3A%2F%2Flocal%3A80%2Ffile%2F%252Fvar%252Flog%0A"
            ).unwrap()).unwrap(),
            "http://cache/http%3A%2F%2Flocal%3A80%2Ffile%2F%252Fvar%252Flog%0A".to_string()
        );
    }
}