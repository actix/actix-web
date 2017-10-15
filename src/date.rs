use std::cell::RefCell;
use std::fmt::{self, Write};
use std::str;

use time::{self, Duration};
use bytes::BytesMut;

// "Sun, 06 Nov 1994 08:49:37 GMT".len()
pub const DATE_VALUE_LENGTH: usize = 29;

pub fn extend(dst: &mut BytesMut) {
    CACHED.with(|cache| {
        let mut cache = cache.borrow_mut();
        let now = time::get_time();
        if now > cache.next_update {
            cache.update(now);
        }
        dst.extend_from_slice(cache.buffer());
    })
}

struct CachedDate {
    bytes: [u8; DATE_VALUE_LENGTH],
    pos: usize,
    next_update: time::Timespec,
}

thread_local!(static CACHED: RefCell<CachedDate> = RefCell::new(CachedDate {
    bytes: [0; DATE_VALUE_LENGTH],
    pos: 0,
    next_update: time::Timespec::new(0, 0),
}));

impl CachedDate {
    fn buffer(&self) -> &[u8] {
        &self.bytes[..]
    }

    fn update(&mut self, now: time::Timespec) {
        self.pos = 0;
        write!(self, "{}", time::at_utc(now).rfc822()).unwrap();
        assert_eq!(self.pos, DATE_VALUE_LENGTH);
        self.next_update = now + Duration::seconds(1);
        self.next_update.nsec = 0;
    }
}

impl fmt::Write for CachedDate {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let len = s.len();
        self.bytes[self.pos..self.pos + len].copy_from_slice(s.as_bytes());
        self.pos += len;
        Ok(())
    }
}

#[test]
fn test_date_len() {
    assert_eq!(DATE_VALUE_LENGTH, "Sun, 06 Nov 1994 08:49:37 GMT".len());
}

#[test]
fn test_date() {
    let mut buf1 = BytesMut::new();
    extend(&mut buf1);
    let mut buf2 = BytesMut::new();
    extend(&mut buf2);
    assert_eq!(buf1, buf2);
}
