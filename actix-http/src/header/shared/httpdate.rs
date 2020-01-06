use std::fmt::{self, Display};
use std::io::Write;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::{buf::BufMutExt, BytesMut};
use http::header::{HeaderValue, InvalidHeaderValue};
use time::{PrimitiveDateTime, OffsetDateTime, UtcOffset};

use crate::error::ParseError;
use crate::header::IntoHeaderValue;

/// A timestamp with HTTP formatting and parsing
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HttpDate(OffsetDateTime);

impl FromStr for HttpDate {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<HttpDate, ParseError> {
        match OffsetDateTime::parse(s, "%a, %d %b %Y %H:%M:%S")
            .or_else(|_| OffsetDateTime::parse(s, "%A, %d-%b-%y %H:%M:%S"))
            .or_else(|_| OffsetDateTime::parse(s, "%c"))
        {
            Ok(t) => Ok(HttpDate(t)),
            Err(_) => {
                Err(ParseError::Header)
            },
        }
    }
}

impl Display for HttpDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.format("%a, %d %b %Y %H:%M:%S GMT"), f)
    }
}

impl From<OffsetDateTime> for HttpDate {
    fn from(dt: time::OffsetDateTime) -> HttpDate {
        HttpDate(dt)
    }
}

impl From<SystemTime> for HttpDate {
    fn from(sys: SystemTime) -> HttpDate {
        HttpDate(PrimitiveDateTime::from(sys).using_offset(UtcOffset::UTC))
    }
}

impl IntoHeaderValue for HttpDate {
    type Error = InvalidHeaderValue;

    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        let mut wrt = BytesMut::with_capacity(29).writer();
        write!(wrt, "{}", self.0.format("%a, %d %b %Y %H:%M:%S GMT")).unwrap();
        HeaderValue::from_maybe_shared(wrt.get_mut().split().freeze())
    }
}

impl From<HttpDate> for SystemTime {
    fn from(date: HttpDate) -> SystemTime {
        let dt = date.0;
        let epoch = OffsetDateTime::unix_epoch();

        if dt >= epoch {
            UNIX_EPOCH + (dt - epoch)
        } else {
            UNIX_EPOCH - (epoch - dt)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HttpDate;
    use time::{OffsetDateTime, Date, Time};

    const NOV_07: HttpDate = HttpDate(OffsetDateTime::new(
        Date::try_from_ymd(1994, 11, 7).unwrap(),
        Time::try_from_hms(8, 48, 37).unwrap()
    ));

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
