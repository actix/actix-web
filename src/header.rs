use std::fmt::{self, Display};
use std::io::Write;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use time;
use bytes::{Bytes, BytesMut, BufMut};
use http::{Error as HttpError};
use http::header::{HeaderValue, InvalidHeaderValue, InvalidHeaderValueBytes};

pub use httpresponse::ConnectionType;
pub use cookie::{Cookie, CookieBuilder};
pub use http_range::HttpRange;

use error::ParseError;


pub trait IntoHeaderValue: Sized {
    /// The type returned in the event of a conversion error.
    type Error: Into<HttpError>;

    /// Cast from PyObject to a concrete Python object type.
    fn try_into(self) -> Result<HeaderValue, Self::Error>;
}

impl IntoHeaderValue for HeaderValue {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        Ok(self)
    }
}

impl<'a> IntoHeaderValue for &'a str {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        self.parse()
    }
}

impl<'a> IntoHeaderValue for &'a [u8] {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_bytes(self)
    }
}

impl IntoHeaderValue for Bytes {
    type Error = InvalidHeaderValueBytes;

    #[inline]
    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_shared(self)
    }
}

/// Represents supported types of content encodings
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ContentEncoding {
    /// Automatically select encoding based on encoding negotiation
    Auto,
    /// A format using the Brotli algorithm
    Br,
    /// A format using the zlib structure with deflate algorithm
    Deflate,
    /// Gzip algorithm
    Gzip,
    /// Indicates the identity function (i.e. no compression, nor modification)
    Identity,
}

impl ContentEncoding {

    #[inline]
    pub fn is_compression(&self) -> bool {
        match *self {
            ContentEncoding::Identity | ContentEncoding::Auto => false,
            _ => true
        }
    }

    #[inline]
    pub fn as_str(&self) -> &'static str {
        match *self {
            ContentEncoding::Br => "br",
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Deflate => "deflate",
            ContentEncoding::Identity | ContentEncoding::Auto => "identity",
        }
    }
    /// default quality value
    pub fn quality(&self) -> f64 {
        match *self {
            ContentEncoding::Br => 1.1,
            ContentEncoding::Gzip => 1.0,
            ContentEncoding::Deflate => 0.9,
            ContentEncoding::Identity | ContentEncoding::Auto => 0.1,
        }
    }
}

// TODO: remove memory allocation
impl<'a> From<&'a str> for ContentEncoding {
    fn from(s: &'a str) -> ContentEncoding {
        match s.trim().to_lowercase().as_ref() {
            "br" => ContentEncoding::Br,
            "gzip" => ContentEncoding::Gzip,
            "deflate" => ContentEncoding::Deflate,
            "identity" => ContentEncoding::Identity,
            _ => ContentEncoding::Auto,
        }
    }
}

/// A timestamp with HTTP formatting and parsing
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date(time::Tm);

impl FromStr for Date {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Date, ParseError> {
        match time::strptime(s, "%a, %d %b %Y %T %Z").or_else(|_| {
            time::strptime(s, "%A, %d-%b-%y %T %Z")
        }).or_else(|_| {
            time::strptime(s, "%c")
        }) {
            Ok(t) => Ok(Date(t)),
            Err(_) => Err(ParseError::Header),
        }
    }
}

impl Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0.to_utc().rfc822(), f)
    }
}

impl From<SystemTime> for Date {
    fn from(sys: SystemTime) -> Date {
        let tmspec = match sys.duration_since(UNIX_EPOCH) {
            Ok(dur) => {
                time::Timespec::new(dur.as_secs() as i64, dur.subsec_nanos() as i32)
            },
            Err(err) => {
                let neg = err.duration();
                time::Timespec::new(-(neg.as_secs() as i64), -(neg.subsec_nanos() as i32))
            },
        };
        Date(time::at_utc(tmspec))
    }
}

impl IntoHeaderValue for Date {
    type Error = InvalidHeaderValueBytes;

    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        let mut wrt = BytesMut::with_capacity(29).writer();
        write!(wrt, "{}", self.0.rfc822()).unwrap();
        HeaderValue::from_shared(wrt.get_mut().take().freeze())
    }
}

impl From<Date> for SystemTime {
    fn from(date: Date) -> SystemTime {
        let spec = date.0.to_timespec();
        if spec.sec >= 0 {
            UNIX_EPOCH + Duration::new(spec.sec as u64, spec.nsec as u32)
        } else {
            UNIX_EPOCH - Duration::new(spec.sec as u64, spec.nsec as u32)
        }
    }
}

#[cfg(test)]
mod tests {
    use time::Tm;
    use super::Date;

    const NOV_07: HttpDate = HttpDate(Tm {
        tm_nsec: 0, tm_sec: 37, tm_min: 48, tm_hour: 8, tm_mday: 7, tm_mon: 10, tm_year: 94,
        tm_wday: 0, tm_isdst: 0, tm_yday: 0, tm_utcoff: 0});

    #[test]
    fn test_date() {
        assert_eq!("Sun, 07 Nov 1994 08:48:37 GMT".parse::<Date>().unwrap(), NOV_07);
        assert_eq!("Sunday, 07-Nov-94 08:48:37 GMT".parse::<Date>().unwrap(), NOV_07);
        assert_eq!("Sun Nov  7 08:48:37 1994".parse::<Date>().unwrap(), NOV_07);
        assert!("this-is-no-date".parse::<Date>().is_err());
    }
}
