use std::fmt::{self, Display, Write};

use super::{
    from_one_raw_str, EntityTag, Header, HeaderName, HeaderValue, HttpDate, InvalidHeaderValue,
    TryIntoHeaderValue, Writer,
};
use crate::{error::ParseError, http::header, HttpMessage};

/// `If-Range` header, defined
/// in [RFC 7233 §3.2](https://datatracker.ietf.org/doc/html/rfc7233#section-3.2)
///
/// If a client has a partial copy of a representation and wishes to have
/// an up-to-date copy of the entire representation, it could use the
/// Range header field with a conditional GET (using either or both of
/// If-Unmodified-Since and If-Match.)  However, if the precondition
/// fails because the representation has been modified, the client would
/// then have to make a second request to obtain the entire current
/// representation.
///
/// The `If-Range` header field allows a client to \"short-circuit\" the
/// second request.  Informally, its meaning is as follows: if the
/// representation is unchanged, send me the part(s) that I am requesting
/// in Range; otherwise, send me the entire representation.
///
/// # ABNF
/// ```plain
/// If-Range = entity-tag / HTTP-date
/// ```
///
/// # Example Values
///
/// * `Sat, 29 Oct 1994 19:43:31 GMT`
/// * `\"xyzzy\"`
///
/// # Examples
/// ```
/// use actix_web::HttpResponse;
/// use actix_web::http::header::{EntityTag, IfRange};
///
/// let mut builder = HttpResponse::Ok();
/// builder.insert_header(
///     IfRange::EntityTag(
///         EntityTag::new(false, "abc".to_owned())
///     )
/// );
/// ```
///
/// ```
/// use std::time::{Duration, SystemTime};
/// use actix_web::{http::header::IfRange, HttpResponse};
///
/// let mut builder = HttpResponse::Ok();
/// let fetched = SystemTime::now() - Duration::from_secs(60 * 60 * 24);
/// builder.insert_header(
///     IfRange::Date(fetched.into())
/// );
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IfRange {
    /// The entity-tag the client has of the resource.
    EntityTag(EntityTag),

    /// The date when the client retrieved the resource.
    Date(HttpDate),
}

impl Header for IfRange {
    fn name() -> HeaderName {
        header::IF_RANGE
    }
    #[inline]
    fn parse<T>(msg: &T) -> Result<Self, ParseError>
    where
        T: HttpMessage,
    {
        let etag: Result<EntityTag, _> = from_one_raw_str(msg.headers().get(&header::IF_RANGE));
        if let Ok(etag) = etag {
            return Ok(IfRange::EntityTag(etag));
        }
        let date: Result<HttpDate, _> = from_one_raw_str(msg.headers().get(&header::IF_RANGE));
        if let Ok(date) = date {
            return Ok(IfRange::Date(date));
        }
        Err(ParseError::Header)
    }
}

impl Display for IfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            IfRange::EntityTag(ref x) => Display::fmt(x, f),
            IfRange::Date(ref x) => Display::fmt(x, f),
        }
    }
}

impl TryIntoHeaderValue for IfRange {
    type Error = InvalidHeaderValue;

    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        let mut writer = Writer::new();
        let _ = write!(&mut writer, "{}", self);
        HeaderValue::from_maybe_shared(writer.take())
    }
}

#[cfg(test)]
mod test_parse_and_format {
    use std::str;

    use super::IfRange as HeaderField;
    use crate::http::header::*;

    crate::http::header::common_header_test!(test1, [b"Sat, 29 Oct 1994 19:43:31 GMT"]);
    crate::http::header::common_header_test!(test2, [b"\"abc\""]);
    crate::http::header::common_header_test!(test3, [b"this-is-invalid"], None::<IfRange>);
}
