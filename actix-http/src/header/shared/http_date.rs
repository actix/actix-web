use std::{fmt, io::Write, str::FromStr, time::SystemTime};

use bytes::BytesMut;
use http::header::{HeaderValue, InvalidHeaderValue};

use crate::{
    date::DATE_VALUE_LENGTH, error::ParseError, header::TryIntoHeaderValue, helpers::MutWriter,
};

/// A timestamp with HTTP-style formatting and parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HttpDate(SystemTime);

impl FromStr for HttpDate {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<HttpDate, ParseError> {
        match httpdate::parse_http_date(s) {
            Ok(sys_time) => Ok(HttpDate(sys_time)),
            Err(_) => Err(ParseError::Header),
        }
    }
}

impl fmt::Display for HttpDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        httpdate::HttpDate::from(self.0).fmt(f)
    }
}

impl TryIntoHeaderValue for HttpDate {
    type Error = InvalidHeaderValue;

    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        let mut buf = BytesMut::with_capacity(DATE_VALUE_LENGTH);
        let mut wrt = MutWriter(&mut buf);

        // unwrap: date output is known to be well formed and of known length
        write!(wrt, "{}", self).unwrap();

        HeaderValue::from_maybe_shared(buf.split().freeze())
    }
}

impl From<SystemTime> for HttpDate {
    fn from(sys_time: SystemTime) -> HttpDate {
        HttpDate(sys_time)
    }
}

impl From<HttpDate> for SystemTime {
    fn from(HttpDate(sys_time): HttpDate) -> SystemTime {
        sys_time
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn date_header() {
        macro_rules! assert_parsed_date {
            ($case:expr, $exp:expr) => {
                assert_eq!($case.parse::<HttpDate>().unwrap(), $exp);
            };
        }

        // 784198117 = SystemTime::from(datetime!(1994-11-07 08:48:37).assume_utc()).duration_since(SystemTime::UNIX_EPOCH));
        let nov_07 = HttpDate(SystemTime::UNIX_EPOCH + Duration::from_secs(784198117));

        assert_parsed_date!("Mon, 07 Nov 1994 08:48:37 GMT", nov_07);
        assert_parsed_date!("Monday, 07-Nov-94 08:48:37 GMT", nov_07);
        assert_parsed_date!("Mon Nov  7 08:48:37 1994", nov_07);

        assert!("this-is-no-date".parse::<HttpDate>().is_err());
    }
}
