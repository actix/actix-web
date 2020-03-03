use std::fmt::{self, Display};
use std::io::Write;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::{buf::BufMutExt, BytesMut};
use http::header::{HeaderValue, InvalidHeaderValue};
use time::{offset, OffsetDateTime, PrimitiveDateTime};

use crate::error::ParseError;
use crate::header::IntoHeaderValue;
use crate::time_parser;

/// A timestamp with HTTP formatting and parsing
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HttpDate(OffsetDateTime);

impl FromStr for HttpDate {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<HttpDate, ParseError> {
        match time_parser::parse_http_date(s) {
            Some(t) => Ok(HttpDate(t.assume_utc())),
            None => Err(ParseError::Header),
        }
    }
}

impl Display for HttpDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.format("%a, %d %b %Y %H:%M:%S GMT"), f)
    }
}

impl From<OffsetDateTime> for HttpDate {
    fn from(dt: OffsetDateTime) -> HttpDate {
        HttpDate(dt)
    }
}

impl From<SystemTime> for HttpDate {
    fn from(sys: SystemTime) -> HttpDate {
        HttpDate(PrimitiveDateTime::from(sys).assume_utc())
    }
}

impl IntoHeaderValue for HttpDate {
    type Error = InvalidHeaderValue;

    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        let mut wrt = BytesMut::with_capacity(29).writer();
        write!(
            wrt,
            "{}",
            self.0
                .to_offset(offset!(UTC))
                .format("%a, %d %b %Y %H:%M:%S GMT")
        )
        .unwrap();
        HeaderValue::from_maybe_shared(wrt.get_mut().split().freeze())
    }
}

impl From<HttpDate> for SystemTime {
    fn from(date: HttpDate) -> SystemTime {
        let dt = date.0;
        let epoch = OffsetDateTime::unix_epoch();

        UNIX_EPOCH + (dt - epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::HttpDate;
    use time::{date, time, PrimitiveDateTime};

    #[test]
    fn test_date() {
        let nov_07 = HttpDate(
            PrimitiveDateTime::new(date!(1994 - 11 - 07), time!(8:48:37)).assume_utc(),
        );

        assert_eq!(
            "Sun, 07 Nov 1994 08:48:37 GMT".parse::<HttpDate>().unwrap(),
            nov_07
        );
        assert_eq!(
            "Sunday, 07-Nov-94 08:48:37 GMT"
                .parse::<HttpDate>()
                .unwrap(),
            nov_07
        );
        assert_eq!(
            "Sun Nov  7 08:48:37 1994".parse::<HttpDate>().unwrap(),
            nov_07
        );
        assert!("this-is-no-date".parse::<HttpDate>().is_err());
    }
}
