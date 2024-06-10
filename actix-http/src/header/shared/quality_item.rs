use std::{cmp, fmt, str};

use super::Quality;
use crate::error::ParseError;

/// Represents an item with a quality value as defined
/// in [RFC 7231 §5.3.1](https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.1).
///
/// # Parsing and Formatting
/// This wrapper be used to parse header value items that have a q-factor annotation as well as
/// serialize items with a their q-factor.
///
/// # Ordering
/// Since this context of use for this type is header value items, ordering is defined for
/// `QualityItem`s but _only_ considers the item's quality. Order of appearance should be used as
/// the secondary sorting parameter; i.e., a stable sort over the quality values will produce a
/// correctly sorted sequence.
///
/// # Examples
/// ```
/// # use actix_http::header::{QualityItem, q};
/// let q_item: QualityItem<String> = "hello;q=0.3".parse().unwrap();
/// assert_eq!(&q_item.item, "hello");
/// assert_eq!(q_item.quality, q(0.3));
///
/// // note that format is normalized compared to parsed item
/// assert_eq!(q_item.to_string(), "hello; q=0.3");
///
/// // item with q=0.3 is greater than item with q=0.1
/// let q_item_fallback: QualityItem<String> = "abc;q=0.1".parse().unwrap();
/// assert!(q_item > q_item_fallback);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualityItem<T> {
    /// The wrapped contents of the field.
    pub item: T,

    /// The quality (client or server preference) for the value.
    pub quality: Quality,
}

impl<T> QualityItem<T> {
    /// Constructs a new `QualityItem` from an item and a quality value.
    ///
    /// The item can be of any type. The quality should be a value in the range [0, 1].
    pub fn new(item: T, quality: Quality) -> Self {
        QualityItem { item, quality }
    }

    /// Constructs a new `QualityItem` from an item, using the maximum q-value.
    pub fn max(item: T) -> Self {
        Self::new(item, Quality::MAX)
    }

    /// Constructs a new `QualityItem` from an item, using the minimum, non-zero q-value.
    pub fn min(item: T) -> Self {
        Self::new(item, Quality::MIN)
    }

    /// Constructs a new `QualityItem` from an item, using zero q-value of zero.
    pub fn zero(item: T) -> Self {
        Self::new(item, Quality::ZERO)
    }
}

impl<T: PartialEq> PartialOrd for QualityItem<T> {
    fn partial_cmp(&self, other: &QualityItem<T>) -> Option<cmp::Ordering> {
        self.quality.partial_cmp(&other.quality)
    }
}

impl<T: fmt::Display> fmt::Display for QualityItem<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.item, f)?;

        match self.quality {
            // q-factor value is implied for max value
            Quality::MAX => Ok(()),

            // fast path for zero
            Quality::ZERO => f.write_str("; q=0"),

            // quality formatting is already using itoa
            q => write!(f, "; q={}", q),
        }
    }
}

impl<T: str::FromStr> str::FromStr for QualityItem<T> {
    type Err = ParseError;

    fn from_str(q_item_str: &str) -> Result<Self, Self::Err> {
        if !q_item_str.is_ascii() {
            return Err(ParseError::Header);
        }

        // set defaults used if quality-item parsing fails, i.e., item has no q attribute
        let mut raw_item = q_item_str;
        let mut quality = Quality::MAX;

        let parts = q_item_str
            .rsplit_once(';')
            .map(|(item, q_attr)| (item.trim(), q_attr.trim()));

        if let Some((val, q_attr)) = parts {
            // example for item with q-factor:
            //
            // gzip;q=0.65
            // ^^^^         val
            //      ^^^^^^  q_attr
            //      ^^      q
            //        ^^^^  q_val

            if q_attr.len() < 2 {
                // Can't possibly be an attribute since an attribute needs at least a name followed
                // by an equals sign. And bare identifiers are forbidden.
                return Err(ParseError::Header);
            }

            let q = &q_attr[0..2];

            if q == "q=" || q == "Q=" {
                let q_val = &q_attr[2..];
                if q_val.len() > 5 {
                    // longer than 5 indicates an over-precise q-factor
                    return Err(ParseError::Header);
                }

                let q_value = q_val.parse::<f32>().map_err(|_| ParseError::Header)?;
                let q_value = Quality::try_from(q_value).map_err(|_| ParseError::Header)?;

                quality = q_value;
                raw_item = val;
            }
        }

        let item = raw_item.parse::<T>().map_err(|_| ParseError::Header)?;

        Ok(QualityItem::new(item, quality))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // copy of encoding from actix-web headers
    #[allow(clippy::enum_variant_names)] // allow Encoding prefix on EncodingExt
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Encoding {
        Chunked,
        Brotli,
        Gzip,
        Deflate,
        Compress,
        Identity,
        Trailers,
        EncodingExt(String),
    }

    impl fmt::Display for Encoding {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            use Encoding::*;
            f.write_str(match *self {
                Chunked => "chunked",
                Brotli => "br",
                Gzip => "gzip",
                Deflate => "deflate",
                Compress => "compress",
                Identity => "identity",
                Trailers => "trailers",
                EncodingExt(ref s) => s.as_ref(),
            })
        }
    }

    impl str::FromStr for Encoding {
        type Err = crate::error::ParseError;
        fn from_str(s: &str) -> Result<Encoding, crate::error::ParseError> {
            use Encoding::*;
            match s {
                "chunked" => Ok(Chunked),
                "br" => Ok(Brotli),
                "deflate" => Ok(Deflate),
                "gzip" => Ok(Gzip),
                "compress" => Ok(Compress),
                "identity" => Ok(Identity),
                "trailers" => Ok(Trailers),
                _ => Ok(EncodingExt(s.to_owned())),
            }
        }
    }

    #[test]
    fn test_quality_item_fmt_q_1() {
        use Encoding::*;
        let x = QualityItem::max(Chunked);
        assert_eq!(format!("{}", x), "chunked");
    }
    #[test]
    fn test_quality_item_fmt_q_0001() {
        use Encoding::*;
        let x = QualityItem::new(Chunked, Quality(1));
        assert_eq!(format!("{}", x), "chunked; q=0.001");
    }
    #[test]
    fn test_quality_item_fmt_q_05() {
        use Encoding::*;
        // Custom value
        let x = QualityItem {
            item: EncodingExt("identity".to_owned()),
            quality: Quality(500),
        };
        assert_eq!(format!("{}", x), "identity; q=0.5");
    }

    #[test]
    fn test_quality_item_fmt_q_0() {
        use Encoding::*;
        // Custom value
        let x = QualityItem {
            item: EncodingExt("identity".to_owned()),
            quality: Quality(0),
        };
        assert_eq!(x.to_string(), "identity; q=0");
    }

    #[test]
    fn test_quality_item_from_str1() {
        use Encoding::*;
        let x: Result<QualityItem<Encoding>, _> = "chunked".parse();
        assert_eq!(
            x.unwrap(),
            QualityItem {
                item: Chunked,
                quality: Quality(1000),
            }
        );
    }

    #[test]
    fn test_quality_item_from_str2() {
        use Encoding::*;
        let x: Result<QualityItem<Encoding>, _> = "chunked; q=1".parse();
        assert_eq!(
            x.unwrap(),
            QualityItem {
                item: Chunked,
                quality: Quality(1000),
            }
        );
    }

    #[test]
    fn test_quality_item_from_str3() {
        use Encoding::*;
        let x: Result<QualityItem<Encoding>, _> = "gzip; q=0.5".parse();
        assert_eq!(
            x.unwrap(),
            QualityItem {
                item: Gzip,
                quality: Quality(500),
            }
        );
    }

    #[test]
    fn test_quality_item_from_str4() {
        use Encoding::*;
        let x: Result<QualityItem<Encoding>, _> = "gzip; q=0.273".parse();
        assert_eq!(
            x.unwrap(),
            QualityItem {
                item: Gzip,
                quality: Quality(273),
            }
        );
    }

    #[test]
    fn test_quality_item_from_str5() {
        let x: Result<QualityItem<Encoding>, _> = "gzip; q=0.2739999".parse();
        assert!(x.is_err());
    }

    #[test]
    fn test_quality_item_from_str6() {
        let x: Result<QualityItem<Encoding>, _> = "gzip; q=2".parse();
        assert!(x.is_err());
    }

    #[test]
    fn test_quality_item_ordering() {
        let x: QualityItem<Encoding> = "gzip; q=0.5".parse().ok().unwrap();
        let y: QualityItem<Encoding> = "gzip; q=0.273".parse().ok().unwrap();
        let comparison_result: bool = x.gt(&y);
        assert!(comparison_result)
    }

    #[test]
    fn test_fuzzing_bugs() {
        assert!("99999;".parse::<QualityItem<String>>().is_err());
        assert!("\x0d;;;=\u{d6aa}==".parse::<QualityItem<String>>().is_err())
    }
}
