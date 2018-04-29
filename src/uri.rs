use http::Uri;

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

#[inline]
fn bit_at(array: &[u8], ch: u8) -> bool {
    array[(ch >> 3) as usize] & (1 << (ch & 7)) != 0
}

#[inline]
fn set_bit(array: &mut [u8], ch: u8) {
    array[(ch >> 3) as usize] |= 1 << (ch & 7)
}

lazy_static! {
    static ref DEFAULT_QUOTER: Quoter = { Quoter::new(b"@:", b"/+") };
}

#[derive(Default)]
pub(crate) struct Url {
    uri: Uri,
    path: Option<String>,
}

impl Url {
    pub fn new(uri: Uri) -> Url {
        let path = DEFAULT_QUOTER.requote(uri.path().as_bytes());

        Url {
            uri,
            path,
        }
    }

    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    pub fn uri_mut(&mut self) -> &mut Uri {
        &mut self.uri
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
    protected_table: [u8; 16],
}

impl Quoter {
    pub fn new(safe: &[u8], protected: &[u8]) -> Quoter {
        let mut q = Quoter {
            safe_table: [0; 16],
            protected_table: [0; 16],
        };

        // prepare safe table
        for i in 0..128 {
            if ALLOWED.contains(&i) {
                set_bit(&mut q.safe_table, i);
            }
            if QS.contains(&i) {
                set_bit(&mut q.safe_table, i);
            }
        }

        for ch in safe {
            set_bit(&mut q.safe_table, *ch)
        }

        // prepare protected table
        for ch in protected {
            set_bit(&mut q.safe_table, *ch);
            set_bit(&mut q.protected_table, *ch);
        }

        q
    }

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

                    if let Some(ch) = restore_ch(pct[1], pct[2]) {
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

        if let Some(data) = cloned {
            Some(unsafe { String::from_utf8_unchecked(data) })
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
