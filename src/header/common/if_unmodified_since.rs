use http::header;

use header::{Header, HttpDate, IntoHeaderValue};
use error::ParseError;
use httpmessage::HttpMessage;


#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct IfUnmodifiedSince(pub HttpDate);

impl Header for IfUnmodifiedSince {
    fn name() -> header::HeaderName {
        header::IF_MODIFIED_SINCE
    }
    
    fn parse<T: HttpMessage>(msg: &T) -> Result<Self, ParseError> {
        let val = msg.headers().get(Self::name())
            .ok_or(ParseError::Header)?.to_str().map_err(|_| ParseError::Header)?;
        Ok(IfUnmodifiedSince(val.parse()?))
    }
}

impl IntoHeaderValue for IfUnmodifiedSince {
    type Error = header::InvalidHeaderValueBytes;

    fn try_into(self) -> Result<header::HeaderValue, Self::Error> {
        self.0.try_into()
    }
}

#[cfg(test)]
mod tests {
    use time::Tm;
    use test::TestRequest;
    use httpmessage::HttpMessage;
    use super::HttpDate;
    use super::IfUnmodifiedSince;

    fn date() -> HttpDate {
        Tm {
            tm_nsec: 0, tm_sec: 37, tm_min: 48, tm_hour: 8,
            tm_mday: 7, tm_mon: 10, tm_year: 94,
            tm_wday: 0, tm_isdst: 0, tm_yday: 0, tm_utcoff: 0}.into()
    }

    #[test]
    fn test_if_mod_since() {
        let req = TestRequest::with_hdr(IfUnmodifiedSince(date())).finish();
        let h = req.get::<IfUnmodifiedSince>().unwrap();
        assert_eq!(h.0, date());
    }
}
