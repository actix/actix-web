use std::{fmt, str};

/// A value to represent an encoding used in the `Accept-Encoding` header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// The no-op "identity" encoding.
    Identity,

    /// Brotli compression (`br`).
    Brotli,

    /// Gzip compression.
    Gzip,

    /// Deflate (LZ77) encoding.
    Deflate,

    /// Zstd compression.
    Zstd,

    /// Some other encoding that is less common, can be any String.
    Other(String),
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Encoding::Identity => "identity",
            Encoding::Brotli => "br",
            Encoding::Gzip => "gzip",
            Encoding::Deflate => "deflate",
            Encoding::Zstd => "zstd",
            Encoding::Other(ref enc) => enc.as_ref(),
        })
    }
}

impl str::FromStr for Encoding {
    type Err = crate::error::ParseError;

    fn from_str(enc_str: &str) -> Result<Self, crate::error::ParseError> {
        match enc_str {
            "identity" => Ok(Self::Identity),
            "br" => Ok(Self::Brotli),
            "gzip" => Ok(Self::Gzip),
            "deflate" => Ok(Self::Deflate),
            "zstd" => Ok(Self::Zstd),
            _ => Ok(Self::Other(enc_str.to_owned())),
        }
    }
}
