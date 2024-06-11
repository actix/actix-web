use std::fmt;

use derive_more::{Display, Error};

const MAX_QUALITY_INT: u16 = 1000;
const MAX_QUALITY_FLOAT: f32 = 1.0;

/// Represents a quality used in q-factor values.
///
/// The default value is equivalent to `q=1.0` (the [max](Self::MAX) value).
///
/// # Implementation notes
/// The quality value is defined as a number between 0.0 and 1.0 with three decimal places.
/// This means there are 1001 possible values. Since floating point numbers are not exact and the
/// smallest floating point data type (`f32`) consumes four bytes, we use an `u16` value to store
/// the quality internally.
///
/// [RFC 7231 §5.3.1] gives more information on quality values in HTTP header fields.
///
/// # Examples
/// ```
/// use actix_http::header::{Quality, q};
/// assert_eq!(q(1.0), Quality::MAX);
///
/// assert_eq!(q(0.42).to_string(), "0.42");
/// assert_eq!(q(1.0).to_string(), "1");
/// assert_eq!(Quality::MIN.to_string(), "0.001");
/// assert_eq!(Quality::ZERO.to_string(), "0");
/// ```
///
/// [RFC 7231 §5.3.1]: https://datatracker.ietf.org/doc/html/rfc7231#section-5.3.1
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Quality(pub(super) u16);

impl Quality {
    /// The maximum quality value, equivalent to `q=1.0`.
    pub const MAX: Quality = Quality(MAX_QUALITY_INT);

    /// The minimum, non-zero quality value, equivalent to `q=0.001`.
    pub const MIN: Quality = Quality(1);

    /// The zero quality value, equivalent to `q=0.0`.
    pub const ZERO: Quality = Quality(0);

    /// Converts a float in the range 0.0–1.0 to a `Quality`.
    ///
    /// Intentionally private. External uses should rely on the `TryFrom` impl.
    ///
    /// # Panics
    /// Panics in debug mode when value is not in the range 0.0 <= n <= 1.0.
    fn from_f32(value: f32) -> Self {
        // Check that `value` is within range should be done before calling this method.
        // Just in case, this debug_assert should catch if we were forgetful.
        debug_assert!(
            (0.0..=MAX_QUALITY_FLOAT).contains(&value),
            "q value must be between 0.0 and 1.0"
        );

        Quality((value * MAX_QUALITY_INT as f32) as u16)
    }
}

/// The default value is [`Quality::MAX`].
impl Default for Quality {
    fn default() -> Quality {
        Quality::MAX
    }
}

impl fmt::Display for Quality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            0 => f.write_str("0"),
            MAX_QUALITY_INT => f.write_str("1"),

            // some number in the range 1–999
            x => {
                f.write_str("0.")?;

                // This implementation avoids string allocation for removing trailing zeroes.
                // In benchmarks it is twice as fast as approach using something like
                // `format!("{}").trim_end_matches('0')` for non-fast-path quality values.

                if x < 10 {
                    // x in is range 1–9

                    f.write_str("00")?;

                    // 0 is already handled so it's not possible to have a trailing 0 in this range
                    // we can just write the integer
                    itoa_fmt(f, x)
                } else if x < 100 {
                    // x in is range 10–99

                    f.write_str("0")?;

                    if x % 10 == 0 {
                        // trailing 0, divide by 10 and write
                        itoa_fmt(f, x / 10)
                    } else {
                        itoa_fmt(f, x)
                    }
                } else {
                    // x is in range 100–999

                    if x % 100 == 0 {
                        // two trailing 0s, divide by 100 and write
                        itoa_fmt(f, x / 100)
                    } else if x % 10 == 0 {
                        // one trailing 0, divide by 10 and write
                        itoa_fmt(f, x / 10)
                    } else {
                        itoa_fmt(f, x)
                    }
                }
            }
        }
    }
}

/// Write integer to a `fmt::Write`.
pub fn itoa_fmt<W: fmt::Write, V: itoa::Integer>(mut wr: W, value: V) -> fmt::Result {
    let mut buf = itoa::Buffer::new();
    wr.write_str(buf.format(value))
}

#[derive(Debug, Clone, Display, Error)]
#[display(fmt = "quality out of bounds")]
#[non_exhaustive]
pub struct QualityOutOfBounds;

impl TryFrom<f32> for Quality {
    type Error = QualityOutOfBounds;

    #[inline]
    fn try_from(value: f32) -> Result<Self, Self::Error> {
        if (0.0..=MAX_QUALITY_FLOAT).contains(&value) {
            Ok(Quality::from_f32(value))
        } else {
            Err(QualityOutOfBounds)
        }
    }
}

/// Convenience function to create a [`Quality`] from an `f32` (0.0–1.0).
///
/// Not recommended for use with user input. Rely on the `TryFrom` impls where possible.
///
/// # Panics
/// Panics if value is out of range.
///
/// # Examples
/// ```
/// # use actix_http::header::{q, Quality};
/// let q1 = q(1.0);
/// assert_eq!(q1, Quality::MAX);
///
/// let q2 = q(0.001);
/// assert_eq!(q2, Quality::MIN);
///
/// let q3 = q(0.0);
/// assert_eq!(q3, Quality::ZERO);
///
/// let q4 = q(0.42);
/// ```
///
/// An out-of-range `f32` quality will panic.
/// ```should_panic
/// # use actix_http::header::q;
/// let _q2 = q(1.42);
/// ```
#[inline]
pub fn q<T>(quality: T) -> Quality
where
    T: TryInto<Quality>,
    T::Error: fmt::Debug,
{
    quality.try_into().expect("quality value was out of bounds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_helper() {
        assert_eq!(q(0.5), Quality(500));
    }

    #[test]
    fn display_output() {
        assert_eq!(Quality::ZERO.to_string(), "0");
        assert_eq!(Quality::MIN.to_string(), "0.001");
        assert_eq!(Quality::MAX.to_string(), "1");

        assert_eq!(q(0.0).to_string(), "0");
        assert_eq!(q(1.0).to_string(), "1");
        assert_eq!(q(0.001).to_string(), "0.001");
        assert_eq!(q(0.5).to_string(), "0.5");
        assert_eq!(q(0.22).to_string(), "0.22");
        assert_eq!(q(0.123).to_string(), "0.123");
        assert_eq!(q(0.999).to_string(), "0.999");

        for x in 0..=1000 {
            // if trailing zeroes are handled correctly, we would not expect the serialized length
            // to ever exceed "0." + 3 decimal places = 5 in length
            assert!(q(x as f32 / 1000.0).to_string().len() <= 5);
        }
    }

    #[test]
    #[should_panic]
    fn negative_quality() {
        q(-1.0);
    }

    #[test]
    #[should_panic]
    fn quality_out_of_bounds() {
        q(2.0);
    }
}
