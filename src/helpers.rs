use std::{str, mem, ptr, slice};
use std::cell::RefCell;
use std::fmt::{self, Write};
use time;
use bytes::BytesMut;
use http::header::HeaderValue;

// "Sun, 06 Nov 1994 08:49:37 GMT".len()
pub const DATE_VALUE_LENGTH: usize = 29;

pub fn date(dst: &mut BytesMut) {
    CACHED.with(|cache| {
        dst.extend_from_slice(cache.borrow().buffer());
    })
}

pub fn update_date() {
    CACHED.with(|cache| {
        cache.borrow_mut().update();
    });
}

struct CachedDate {
    bytes: [u8; DATE_VALUE_LENGTH],
    pos: usize,
}

thread_local!(static CACHED: RefCell<CachedDate> = RefCell::new(CachedDate {
    bytes: [0; DATE_VALUE_LENGTH],
    pos: 0,
}));

impl CachedDate {
    fn buffer(&self) -> &[u8] {
        &self.bytes[..]
    }

    fn update(&mut self) {
        self.pos = 0;
        write!(self, "{}", time::at_utc(time::get_time()).rfc822()).unwrap();
        assert_eq!(self.pos, DATE_VALUE_LENGTH);
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

const DEC_DIGITS_LUT: &[u8] =
    b"0001020304050607080910111213141516171819\
      2021222324252627282930313233343536373839\
      4041424344454647484950515253545556575859\
      6061626364656667686970717273747576777879\
      8081828384858687888990919293949596979899";

pub(crate) fn convert_u16(mut n: u16, bytes: &mut BytesMut) {
    let mut buf: [u8; 39] = unsafe { mem::uninitialized() };
    let mut curr = buf.len() as isize;
    let buf_ptr = buf.as_mut_ptr();
    let lut_ptr = DEC_DIGITS_LUT.as_ptr();

    unsafe {
        // need at least 16 bits for the 4-characters-at-a-time to work.
        if mem::size_of::<u16>() >= 2 {
            // eagerly decode 4 characters at a time
            while n >= 10_000 {
                let rem = (n % 10_000) as isize;
                n /= 10_000;

                let d1 = (rem / 100) << 1;
                let d2 = (rem % 100) << 1;
                curr -= 4;
                ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
                ptr::copy_nonoverlapping(lut_ptr.offset(d2), buf_ptr.offset(curr + 2), 2);
            }
        }

        // if we reach here numbers are <= 9999, so at most 4 chars long
        let mut n = n as isize; // possibly reduce 64bit math

        // decode 2 more chars, if > 2 chars
        if n >= 100 {
            let d1 = (n % 100) << 1;
            n /= 100;
            curr -= 2;
            ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
        }

        // decode last 1 or 2 chars
        if n < 10 {
            curr -= 1;
            *buf_ptr.offset(curr) = (n as u8) + b'0';
        } else {
            let d1 = n << 1;
            curr -= 2;
            ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
        }
    }

    unsafe {
        bytes.extend_from_slice(
            slice::from_raw_parts(buf_ptr.offset(curr), buf.len() - curr as usize));
    }
}

pub(crate) fn convert_into_header(mut n: usize) -> HeaderValue {
    let mut curr: isize = 39;
    let mut buf: [u8; 39] = unsafe { mem::uninitialized() };
    let buf_ptr = buf.as_mut_ptr();
    let lut_ptr = DEC_DIGITS_LUT.as_ptr();

    unsafe {
        // need at least 16 bits for the 4-characters-at-a-time to work.
        if mem::size_of::<usize>() >= 2 {
            // eagerly decode 4 characters at a time
            while n >= 10_000 {
                let rem = (n % 10_000) as isize;
                n /= 10_000;

                let d1 = (rem / 100) << 1;
                let d2 = (rem % 100) << 1;
                curr -= 4;
                ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
                ptr::copy_nonoverlapping(lut_ptr.offset(d2), buf_ptr.offset(curr + 2), 2);
            }
        }

        // if we reach here numbers are <= 9999, so at most 4 chars long
        let mut n = n as isize; // possibly reduce 64bit math

        // decode 2 more chars, if > 2 chars
        if n >= 100 {
            let d1 = (n % 100) << 1;
            n /= 100;
            curr -= 2;
            ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
        }

        // decode last 1 or 2 chars
        if n < 10 {
            curr -= 1;
            *buf_ptr.offset(curr) = (n as u8) + b'0';
        } else {
            let d1 = n << 1;
            curr -= 2;
            ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
        }
    }

    unsafe {
        HeaderValue::from_bytes(
            slice::from_raw_parts(buf_ptr.offset(curr), buf.len() - curr as usize)).unwrap()
    }
}

#[test]
fn test_date_len() {
    assert_eq!(DATE_VALUE_LENGTH, "Sun, 06 Nov 1994 08:49:37 GMT".len());
}

#[test]
fn test_date() {
    let mut buf1 = BytesMut::new();
    date(&mut buf1);
    let mut buf2 = BytesMut::new();
    date(&mut buf2);
    assert_eq!(buf1, buf2);
}
