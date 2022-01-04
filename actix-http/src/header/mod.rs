//! Pre-defined `HeaderName`s, traits for parsing and conversion, and other header utility methods.

use percent_encoding::{AsciiSet, CONTROLS};

// re-export from http except header map related items
pub use http::header::{
    HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue, ToStrError,
};

// re-export const header names
pub use http::header::{
    ACCEPT, ACCEPT_CHARSET, ACCEPT_ENCODING, ACCEPT_LANGUAGE, ACCEPT_RANGES,
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS,
    ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS,
    ACCESS_CONTROL_MAX_AGE, ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD, AGE,
    ALLOW, ALT_SVC, AUTHORIZATION, CACHE_CONTROL, CONNECTION, CONTENT_DISPOSITION,
    CONTENT_ENCODING, CONTENT_LANGUAGE, CONTENT_LENGTH, CONTENT_LOCATION, CONTENT_RANGE,
    CONTENT_SECURITY_POLICY, CONTENT_SECURITY_POLICY_REPORT_ONLY, CONTENT_TYPE, COOKIE, DATE,
    DNT, ETAG, EXPECT, EXPIRES, FORWARDED, FROM, HOST, IF_MATCH, IF_MODIFIED_SINCE,
    IF_NONE_MATCH, IF_RANGE, IF_UNMODIFIED_SINCE, LAST_MODIFIED, LINK, LOCATION, MAX_FORWARDS,
    ORIGIN, PRAGMA, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, PUBLIC_KEY_PINS,
    PUBLIC_KEY_PINS_REPORT_ONLY, RANGE, REFERER, REFERRER_POLICY, REFRESH, RETRY_AFTER,
    SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_EXTENSIONS, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_PROTOCOL,
    SEC_WEBSOCKET_VERSION, SERVER, SET_COOKIE, STRICT_TRANSPORT_SECURITY, TE, TRAILER,
    TRANSFER_ENCODING, UPGRADE, UPGRADE_INSECURE_REQUESTS, USER_AGENT, VARY, VIA, WARNING,
    WWW_AUTHENTICATE, X_CONTENT_TYPE_OPTIONS, X_DNS_PREFETCH_CONTROL, X_FRAME_OPTIONS,
    X_XSS_PROTECTION,
};

use crate::{error::ParseError, HttpMessage};

mod as_name;
mod into_pair;
mod into_value;
pub mod map;
mod shared;
mod utils;

pub use self::as_name::AsHeaderName;
pub use self::into_pair::TryIntoHeaderPair;
pub use self::into_value::TryIntoHeaderValue;
pub use self::map::HeaderMap;
pub use self::shared::{
    parse_extended_value, q, Charset, ContentEncoding, ExtendedValue, HttpDate, LanguageTag,
    Quality, QualityItem,
};
pub use self::utils::{
    fmt_comma_delimited, from_comma_delimited, from_one_raw_str, http_percent_encode,
};

/// An interface for types that already represent a valid header.
pub trait Header: TryIntoHeaderValue {
    /// Returns the name of the header field.
    fn name() -> HeaderName;

    /// Parse the header from a HTTP message.
    fn parse<M: HttpMessage>(msg: &M) -> Result<Self, ParseError>;
}

/// This encode set is used for HTTP header values and is defined at
/// <https://datatracker.ietf.org/doc/html/rfc5987#section-3.2>.
pub(crate) const HTTP_VALUE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'%')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'-')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'{')
    .add(b'}');
