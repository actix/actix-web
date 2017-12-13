use std;
use std::ops::Index;
use std::path::PathBuf;
use std::str::FromStr;
use smallvec::SmallVec;

use error::{ResponseError, UriSegmentError, ErrorBadRequest};


/// A trait to abstract the idea of creating a new instance of a type from a path parameter.
pub trait FromParam: Sized {
    /// The associated error which can be returned from parsing.
    type Err: ResponseError;

    /// Parses a string `s` to return a value of this type.
    fn from_param(s: &str) -> Result<Self, Self::Err>;
}

/// Route match information
///
/// If resource path contains variable patterns, `Params` stores this variables.
#[derive(Debug)]
pub struct Params<'a>(SmallVec<[(&'a str, &'a str); 3]>);

impl<'a> Default for Params<'a> {
    fn default() -> Params<'a> {
        Params(SmallVec::new())
    }
}

impl<'a> Params<'a> {

    pub(crate) fn add(&mut self, name: &'a str, value: &'a str) {
        self.0.push((name, value));
    }

    /// Check if there are any matched patterns
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get matched parameter by name without type conversion
    pub fn get(&self, key: &str) -> Option<&'a str> {
        for item in &self.0 {
            if key == item.0 {
                return Some(item.1)
            }
        }
        None
    }

    /// Get matched `FromParam` compatible parameter by name.
    ///
    /// If keyed parameter is not available empty string is used as default value.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// fn index(req: HttpRequest) -> Result<String> {
    ///    let ivalue: isize = req.match_info().query("val")?;
    ///    Ok(format!("isuze value: {:?}", ivalue))
    /// }
    /// # fn main() {}
    /// ```
    pub fn query<T: FromParam>(&self, key: &str) -> Result<T, <T as FromParam>::Err>
    {
        if let Some(s) = self.get(key) {
            T::from_param(s)
        } else {
            T::from_param("")
        }
    }
}

impl<'a, 'b> Index<&'b str> for Params<'a> {
    type Output = str;

    fn index(&self, name: &'b str) -> &str {
        self.get(name).expect("Value for parameter is not available")
    }
}

/// Creates a `PathBuf` from a path parameter. The returned `PathBuf` is
/// percent-decoded. If a segment is equal to "..", the previous segment (if
/// any) is skipped.
///
/// For security purposes, if a segment meets any of the following conditions,
/// an `Err` is returned indicating the condition met:
///
///   * Decoded segment starts with any of: `.` (except `..`), `*`
///   * Decoded segment ends with any of: `:`, `>`, `<`
///   * Decoded segment contains any of: `/`
///   * On Windows, decoded segment contains any of: '\'
///   * Percent-encoding results in invalid UTF8.
///
/// As a result of these conditions, a `PathBuf` parsed from request path parameter is
/// safe to interpolate within, or use as a suffix of, a path without additional
/// checks.
impl FromParam for PathBuf {
    type Err = UriSegmentError;

    fn from_param(val: &str) -> Result<PathBuf, UriSegmentError> {
        let mut buf = PathBuf::new();
        for segment in val.split('/') {
            if segment == ".." {
                buf.pop();
            } else if segment.starts_with('.') {
                return Err(UriSegmentError::BadStart('.'))
            } else if segment.starts_with('*') {
                return Err(UriSegmentError::BadStart('*'))
            } else if segment.ends_with(':') {
                return Err(UriSegmentError::BadEnd(':'))
            } else if segment.ends_with('>') {
                return Err(UriSegmentError::BadEnd('>'))
            } else if segment.ends_with('<') {
                return Err(UriSegmentError::BadEnd('<'))
            } else if segment.is_empty() {
                continue
            } else if cfg!(windows) && segment.contains('\\') {
                return Err(UriSegmentError::BadChar('\\'))
            } else {
                buf.push(segment)
            }
        }

        Ok(buf)
    }
}

macro_rules! FROM_STR {
    ($type:ty) => {
        impl FromParam for $type {
            type Err = ErrorBadRequest<<$type as FromStr>::Err>;

            fn from_param(val: &str) -> Result<Self, Self::Err> {
                <$type as FromStr>::from_str(val).map_err(ErrorBadRequest)
            }
        }
    }
}

FROM_STR!(u8);
FROM_STR!(u16);
FROM_STR!(u32);
FROM_STR!(u64);
FROM_STR!(usize);
FROM_STR!(i8);
FROM_STR!(i16);
FROM_STR!(i32);
FROM_STR!(i64);
FROM_STR!(isize);
FROM_STR!(f32);
FROM_STR!(f64);
FROM_STR!(String);
FROM_STR!(std::net::IpAddr);
FROM_STR!(std::net::Ipv4Addr);
FROM_STR!(std::net::Ipv6Addr);
FROM_STR!(std::net::SocketAddr);
FROM_STR!(std::net::SocketAddrV4);
FROM_STR!(std::net::SocketAddrV6);

#[cfg(test)]
mod tests {
    use super::*;
    use std::iter::FromIterator;

    #[test]
    fn test_path_buf() {
        assert_eq!(PathBuf::from_param("/test/.tt"), Err(UriSegmentError::BadStart('.')));
        assert_eq!(PathBuf::from_param("/test/*tt"), Err(UriSegmentError::BadStart('*')));
        assert_eq!(PathBuf::from_param("/test/tt:"), Err(UriSegmentError::BadEnd(':')));
        assert_eq!(PathBuf::from_param("/test/tt<"), Err(UriSegmentError::BadEnd('<')));
        assert_eq!(PathBuf::from_param("/test/tt>"), Err(UriSegmentError::BadEnd('>')));
        assert_eq!(PathBuf::from_param("/seg1/seg2/"),
                   Ok(PathBuf::from_iter(vec!["seg1", "seg2"])));
        assert_eq!(PathBuf::from_param("/seg1/../seg2/"),
                   Ok(PathBuf::from_iter(vec!["seg2"])));
    }
}
