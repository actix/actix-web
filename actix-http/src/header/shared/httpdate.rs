use std::fmt::{self, Display};
use std::io::Write;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::{buf::BufMutExt, BytesMut};
use http::header::{HeaderValue, InvalidHeaderValue};

use crate::error::ParseError;
use crate::header::IntoHeaderValue;

/// A timestamp with HTTP formatting and parsing
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HttpDate(time::Tm);

impl FromStr for HttpDate {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<HttpDate, ParseError> {
        match time::strptime(s, "%a, %d %b %Y %T %Z")
            .or_else(|_| time::strptime(s, "%A, %d-%b-%y %T %Z"))
            .or_else(|_| time::strptime(s, "%c"))
        {
            Ok(t) => Ok(HttpDate(t)),
            Err(_) => Err(ParseError::Header),
        }
    }
}

impl Display for HttpDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.to_utc().rfc822(), f)
    }
}

impl From<time::Tm> for HttpDate {
    fn from(tm: time::Tm) -> HttpDate {
        HttpDate(tm)
    }
}

impl From<SystemTime> for HttpDate {
    fn from(sys: SystemTime) -> HttpDate {
        let tmspec = match sys.duration_since(UNIX_EPOCH) {
            Ok(dur) => {
                time::Timespec::new(dur.as_secs() as i64, dur.subsec_nanos() as i32)
            }
            Err(err) => {
                let neg = err.duration();
                time::Timespec::new(
                    -(neg.as_secs() as i64),
                    -(neg.subsec_nanos() as i32),
                )
            }
        };
        HttpDate(time::at_utc(tmspec))
    }
}

impl IntoHeaderValue for HttpDate {
    type Error = InvalidHeaderValue;

    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        let mut wrt = BytesMut::with_capacity(29).writer();
        write!(wrt, "{}", self.0.rfc822()).unwrap();
        HeaderValue::from_maybe_shared(wrt.get_mut().split().freeze())
    }
}

impl From<HttpDate> for SystemTime {
    fn from(date: HttpDate) -> SystemTime {
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
    use super::HttpDate;
    use time::Tm;

    const NOV_07: HttpDate = HttpDate(Tm {
        tm_nsec: 0,
        tm_sec: 37,
        tm_min: 48,
        tm_hour: 8,
        tm_mday: 7,
        tm_mon: 10,
        tm_year: 94,
        tm_wday: 0,
        tm_isdst: 0,
        tm_yday: 0,
        tm_utcoff: 0,
    });

    #[test]
    fn test_date() {
        assert_eq!(
            "Sun, 07 Nov 1994 08:48:37 GMT".parse::<HttpDate>().unwrap(),
            NOV_07
        );
        assert_eq!(
            "Sunday, 07-Nov-94 08:48:37 GMT"
                .parse::<HttpDate>()
                .unwrap(),
            NOV_07
        );
        assert_eq!(
            "Sun Nov  7 08:48:37 1994".parse::<HttpDate>().unwrap(),
            NOV_07
        );
        assert!("this-is-no-date".parse::<HttpDate>().is_err());
    }
}
