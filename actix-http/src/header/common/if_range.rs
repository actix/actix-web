use std::fmt::{self, Display, Write};

use crate::error::ParseError;
use crate::header::{
    self, from_one_raw_str, EntityTag, Header, HeaderName, HeaderValue, HttpDate,
    IntoHeaderValue, InvalidHeaderValue, Writer,
};
use crate::httpmessage::HttpMessage;

/// `If-Range` header, defined in [RFC7233](http://tools.ietf.org/html/rfc7233#section-3.2)
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
///
/// ```text
/// If-Range = entity-tag / HTTP-date
/// ```
///
/// # Example values
///
/// * `Sat, 29 Oct 1994 19:43:31 GMT`
/// * `\"xyzzy\"`
///
/// # Examples
///
/// ```rust
/// use actix_http::Response;
/// use actix_http::http::header::{EntityTag, IfRange};
///
/// let mut builder = Response::Ok();
/// builder.set(IfRange::EntityTag(EntityTag::new(
///     false,
///     "xyzzy".to_owned(),
/// )));
/// ```
///
/// ```rust
/// use actix_http::Response;
/// use actix_http::http::header::IfRange;
/// use std::time::{Duration, SystemTime};
///
/// let mut builder = Response::Ok();
/// let fetched = SystemTime::now() - Duration::from_secs(60 * 60 * 24);
/// builder.set(IfRange::Date(fetched.into()));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum IfRange {
    /// The entity-tag the client has of the resource
    EntityTag(EntityTag),
    /// The date when the client retrieved the resource
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
        let etag: Result<EntityTag, _> =
            from_one_raw_str(msg.headers().get(&header::IF_RANGE));
        if let Ok(etag) = etag {
            return Ok(IfRange::EntityTag(etag));
        }
        let date: Result<HttpDate, _> =
            from_one_raw_str(msg.headers().get(&header::IF_RANGE));
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

impl IntoHeaderValue for IfRange {
    type Error = InvalidHeaderValue;

    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        let mut writer = Writer::new();
        let _ = write!(&mut writer, "{}", self);
        HeaderValue::from_maybe_shared(writer.take())
    }
}

#[cfg(test)]
mod test_if_range {
    use super::IfRange as HeaderField;
    use crate::header::*;
    use std::str;
    test_header!(test1, vec![b"Sat, 29 Oct 1994 19:43:31 GMT"]);
    test_header!(test2, vec![b"\"xyzzy\""]);
    test_header!(test3, vec![b"this-is-invalid"], None::<IfRange>);
}
