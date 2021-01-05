use std::{
    cmp,
    convert::{TryFrom, TryInto},
    fmt, str,
};

use derive_more::{Display, Error};

const MAX_QUALITY: u16 = 1000;
const MAX_FLOAT_QUALITY: f32 = 1.0;

/// Represents a quality used in quality values.
///
/// Can be created with the [`q`] function.
///
/// # Implementation notes
///
/// The quality value is defined as a number between 0 and 1 with three decimal
/// places. This means there are 1001 possible values. Since floating point
/// numbers are not exact and the smallest floating point data type (`f32`)
/// consumes four bytes, hyper uses an `u16` value to store the
/// quality internally. For performance reasons you may set quality directly to
/// a value between 0 and 1000 e.g. `Quality(532)` matches the quality
/// `q=0.532`.
///
/// [RFC7231 Section 5.3.1](https://tools.ietf.org/html/rfc7231#section-5.3.1)
/// gives more information on quality values in HTTP header fields.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Quality(u16);

impl Quality {
    /// # Panics
    /// Panics in debug mode when value is not in the range 0.0 <= n <= 1.0.
    fn from_f32(value: f32) -> Self {
        // Check that `value` is within range should be done before calling this method.
        // Just in case, this debug_assert should catch if we were forgetful.
        debug_assert!(
            (0.0f32..=1.0f32).contains(&value),
            "q value must be between 0.0 and 1.0"
        );

        Quality((value * MAX_QUALITY as f32) as u16)
    }
}

impl Default for Quality {
    fn default() -> Quality {
        Quality(MAX_QUALITY)
    }
}

#[derive(Debug, Clone, Display, Error)]
pub struct QualityOutOfBounds;

impl TryFrom<u16> for Quality {
    type Error = QualityOutOfBounds;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if (0..=MAX_QUALITY).contains(&value) {
            Ok(Quality(value))
        } else {
            Err(QualityOutOfBounds)
        }
    }
}

impl TryFrom<f32> for Quality {
    type Error = QualityOutOfBounds;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        if (0.0..=MAX_FLOAT_QUALITY).contains(&value) {
            Ok(Quality::from_f32(value))
        } else {
            Err(QualityOutOfBounds)
        }
    }
}

/// Represents an item with a quality value as defined in
/// [RFC7231](https://tools.ietf.org/html/rfc7231#section-5.3.1).
#[derive(Clone, PartialEq, Debug)]
pub struct QualityItem<T> {
    /// The actual contents of the field.
    pub item: T,
    /// The quality (client or server preference) for the value.
    pub quality: Quality,
}

impl<T> QualityItem<T> {
    /// Creates a new `QualityItem` from an item and a quality.
    /// The item can be of any type.
    /// The quality should be a value in the range [0, 1].
    pub fn new(item: T, quality: Quality) -> QualityItem<T> {
        QualityItem { item, quality }
    }
}

impl<T: PartialEq> cmp::PartialOrd for QualityItem<T> {
    fn partial_cmp(&self, other: &QualityItem<T>) -> Option<cmp::Ordering> {
        self.quality.partial_cmp(&other.quality)
    }
}

impl<T: fmt::Display> fmt::Display for QualityItem<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.item, f)?;

        match self.quality.0 {
            MAX_QUALITY => Ok(()),
            0 => f.write_str("; q=0"),
            x => write!(f, "; q=0.{}", format!("{:03}", x).trim_end_matches('0')),
        }
    }
}

impl<T: str::FromStr> str::FromStr for QualityItem<T> {
    type Err = crate::error::ParseError;

    fn from_str(qitem_str: &str) -> Result<QualityItem<T>, crate::error::ParseError> {
        if !qitem_str.is_ascii() {
            return Err(crate::error::ParseError::Header);
        }

        // Set defaults used if parsing fails.
        let mut raw_item = qitem_str;
        let mut quality = 1f32;

        let parts: Vec<_> = qitem_str.rsplitn(2, ';').map(str::trim).collect();

        if parts.len() == 2 {
            // example for item with q-factor:
            //
            // gzip; q=0.65
            //       ^^^^^^  parts[0]
            //       ^^      start
            //         ^^^^  q_val
            // ^^^^          parts[1]

            if parts[0].len() < 2 {
                // Can't possibly be an attribute since an attribute needs at least a name followed
                // by an equals sign. And bare identifiers are forbidden.
                return Err(crate::error::ParseError::Header);
            }

            let start = &parts[0][0..2];

            if start == "q=" || start == "Q=" {
                let q_val = &parts[0][2..];
                if q_val.len() > 5 {
                    // longer than 5 indicates an over-precise q-factor
                    return Err(crate::error::ParseError::Header);
                }

                let q_value = q_val
                    .parse::<f32>()
                    .map_err(|_| crate::error::ParseError::Header)?;

                if (0f32..=1f32).contains(&q_value) {
                    quality = q_value;
                    raw_item = parts[1];
                } else {
                    return Err(crate::error::ParseError::Header);
                }
            }
        }

        let item = raw_item
            .parse::<T>()
            .map_err(|_| crate::error::ParseError::Header)?;

        // we already checked above that the quality is within range
        Ok(QualityItem::new(item, Quality::from_f32(quality)))
    }
}

/// Convenience function to wrap a value in a `QualityItem`
/// Sets `q` to the default 1.0
pub fn qitem<T>(item: T) -> QualityItem<T> {
    QualityItem::new(item, Quality::default())
}

/// Convenience function to create a `Quality` from a float or integer.
///
/// Implemented for `u16` and `f32`. Panics if value is out of range.
pub fn q<T>(val: T) -> Quality
where
    T: TryInto<Quality>,
    T::Error: fmt::Debug,
{
    // TODO: on next breaking change, handle unwrap differently
    val.try_into().unwrap()
}

#[cfg(test)]
mod tests {
    use super::super::encoding::*;
    use super::*;

    #[test]
    fn test_quality_item_fmt_q_1() {
        let x = qitem(Chunked);
        assert_eq!(format!("{}", x), "chunked");
    }
    #[test]
    fn test_quality_item_fmt_q_0001() {
        let x = QualityItem::new(Chunked, Quality(1));
        assert_eq!(format!("{}", x), "chunked; q=0.001");
    }
    #[test]
    fn test_quality_item_fmt_q_05() {
        // Custom value
        let x = QualityItem {
            item: EncodingExt("identity".to_owned()),
            quality: Quality(500),
        };
        assert_eq!(format!("{}", x), "identity; q=0.5");
    }

    #[test]
    fn test_quality_item_fmt_q_0() {
        // Custom value
        let x = QualityItem {
            item: EncodingExt("identity".to_owned()),
            quality: Quality(0),
        };
        assert_eq!(x.to_string(), "identity; q=0");
    }

    #[test]
    fn test_quality_item_from_str1() {
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
        let comparision_result: bool = x.gt(&y);
        assert!(comparision_result)
    }

    #[test]
    fn test_quality() {
        assert_eq!(q(0.5), Quality(500));
    }

    #[test]
    #[should_panic]
    fn test_quality_invalid() {
        q(-1.0);
    }

    #[test]
    #[should_panic]
    fn test_quality_invalid2() {
        q(2.0);
    }

    #[test]
    fn test_fuzzing_bugs() {
        assert!("99999;".parse::<QualityItem<String>>().is_err());
        assert!("\x0d;;;=\u{d6aa}==".parse::<QualityItem<String>>().is_err())
    }
}
