use std::{fmt, str};

use actix_http::ContentEncoding;

/// A value to represent an encoding used in the `Accept-Encoding` and `Content-Encoding` header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// A supported content encoding. See [`ContentEncoding`] for variants.
    Known(ContentEncoding),

    /// Some other encoding that is less common, can be any string.
    Unknown(String),
}

impl Encoding {
    pub const fn identity() -> Self {
        Self::Known(ContentEncoding::Identity)
    }

    pub const fn brotli() -> Self {
        Self::Known(ContentEncoding::Brotli)
    }

    pub const fn deflate() -> Self {
        Self::Known(ContentEncoding::Deflate)
    }

    pub const fn gzip() -> Self {
        Self::Known(ContentEncoding::Gzip)
    }

    pub const fn zstd() -> Self {
        Self::Known(ContentEncoding::Zstd)
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Encoding::Known(enc) => enc.as_str(),
            Encoding::Unknown(enc) => enc.as_str(),
        })
    }
}

impl str::FromStr for Encoding {
    type Err = crate::error::ParseError;

    fn from_str(enc: &str) -> Result<Self, crate::error::ParseError> {
        match enc.parse::<ContentEncoding>() {
            Ok(enc) => Ok(Self::Known(enc)),
            Err(_) => Ok(Self::Unknown(enc.to_owned())),
        }
    }
}
