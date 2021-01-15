/// Represents a supported content encoding.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ContentEncoding {
    /// Automatically select encoding based on encoding negotiation.
    Auto,

    /// A format using the Brotli algorithm.
    Br,

    /// A format using the zlib structure with deflate algorithm.
    Deflate,

    /// Gzip algorithm.
    Gzip,

    /// Indicates the identity function (i.e. no compression, nor modification).
    Identity,
}

impl ContentEncoding {
    /// Is the content compressed?
    #[inline]
    pub fn is_compression(self) -> bool {
        matches!(self, ContentEncoding::Identity | ContentEncoding::Auto)
    }

    /// Convert content encoding to string
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            ContentEncoding::Br => "br",
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Deflate => "deflate",
            ContentEncoding::Identity | ContentEncoding::Auto => "identity",
        }
    }

    /// Default Q-factor (quality) value.
    #[inline]
    pub fn quality(self) -> f64 {
        match self {
            ContentEncoding::Br => 1.1,
            ContentEncoding::Gzip => 1.0,
            ContentEncoding::Deflate => 0.9,
            ContentEncoding::Identity | ContentEncoding::Auto => 0.1,
        }
    }
}

impl From<&str> for ContentEncoding {
    fn from(val: &str) -> ContentEncoding {
        let val = val.trim();

        if val.eq_ignore_ascii_case("br") {
            ContentEncoding::Br
        } else if val.eq_ignore_ascii_case("gzip") {
            ContentEncoding::Gzip
        } else if val.eq_ignore_ascii_case("deflate") {
            ContentEncoding::Deflate
        } else {
            ContentEncoding::Identity
        }
    }
}
