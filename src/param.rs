use http::StatusCode;
use smallvec::SmallVec;
use std;
use std::ops::Index;
use std::path::PathBuf;
use std::str::FromStr;

use error::{InternalError, ResponseError, UriSegmentError};
use uri::Url;

/// A trait to abstract the idea of creating a new instance of a type from a
/// path parameter.
pub trait FromParam: Sized {
    /// The associated error which can be returned from parsing.
    type Err: ResponseError;

    /// Parses a string `s` to return a value of this type.
    fn from_param(s: &str) -> Result<Self, Self::Err>;
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ParamItem {
    Static(&'static str),
    UrlSegment(u16, u16),
}

/// Route match information
///
/// If resource path contains variable patterns, `Params` stores this variables.
#[derive(Debug)]
pub struct Params {
    url: Url,
    pub(crate) tail: u16,
    segments: SmallVec<[(&'static str, ParamItem); 3]>,
}

impl Params {
    pub(crate) fn new() -> Params {
        Params {
            url: Url::default(),
            tail: 0,
            segments: SmallVec::new(),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.segments.clear();
    }

    pub(crate) fn set_url(&mut self, url: Url) {
        self.url = url;
    }

    pub(crate) fn set_tail(&mut self, tail: u16) {
        self.tail = tail;
    }

    pub(crate) fn add(&mut self, name: &'static str, value: ParamItem) {
        self.segments.push((name, value));
    }

    pub(crate) fn add_static(&mut self, name: &'static str, value: &'static str) {
        self.segments.push((name, ParamItem::Static(value)));
    }

    /// Check if there are any matched patterns
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Check number of extracted parameters
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Get matched parameter by name without type conversion
    pub fn get(&self, key: &str) -> Option<&str> {
        for item in self.segments.iter() {
            if key == item.0 {
                return match item.1 {
                    ParamItem::Static(s) => Some(s),
                    ParamItem::UrlSegment(s, e) => {
                        Some(&self.url.path()[(s as usize)..(e as usize)])
                    }
                };
            }
        }
        if key == "tail" {
            Some(&self.url.path()[(self.tail as usize)..])
        } else {
            None
        }
    }

    /// Get matched `FromParam` compatible parameter by name.
    ///
    /// If keyed parameter is not available empty string is used as default
    /// value.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// fn index(req: HttpRequest) -> Result<String> {
    ///     let ivalue: isize = req.match_info().query("val")?;
    ///     Ok(format!("isuze value: {:?}", ivalue))
    /// }
    /// # fn main() {}
    /// ```
    pub fn query<T: FromParam>(&self, key: &str) -> Result<T, <T as FromParam>::Err> {
        if let Some(s) = self.get(key) {
            T::from_param(s)
        } else {
            T::from_param("")
        }
    }

    /// Return iterator to items in parameter container
    pub fn iter(&self) -> ParamsIter {
        ParamsIter {
            idx: 0,
            params: self,
        }
    }
}

#[derive(Debug)]
pub struct ParamsIter<'a> {
    idx: usize,
    params: &'a Params,
}

impl<'a> Iterator for ParamsIter<'a> {
    type Item = (&'a str, &'a str);

    #[inline]
    fn next(&mut self) -> Option<(&'a str, &'a str)> {
        if self.idx < self.params.len() {
            let idx = self.idx;
            let res = match self.params.segments[idx].1 {
                ParamItem::Static(s) => s,
                ParamItem::UrlSegment(s, e) => {
                    &self.params.url.path()[(s as usize)..(e as usize)]
                }
            };
            self.idx += 1;
            return Some((self.params.segments[idx].0, res));
        }
        None
    }
}

impl<'a, 'b> Index<&'b str> for &'a Params {
    type Output = str;

    fn index(&self, name: &'b str) -> &str {
        self.get(name)
            .expect("Value for parameter is not available")
    }
}

impl<'a> Index<usize> for &'a Params {
    type Output = str;

    fn index(&self, idx: usize) -> &str {
        match self.segments[idx].1 {
            ParamItem::Static(s) => s,
            ParamItem::UrlSegment(s, e) => &self.url.path()[(s as usize)..(e as usize)],
        }
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
/// As a result of these conditions, a `PathBuf` parsed from request path
/// parameter is safe to interpolate within, or use as a suffix of, a path
/// without additional checks.
impl FromParam for PathBuf {
    type Err = UriSegmentError;

    fn from_param(val: &str) -> Result<PathBuf, UriSegmentError> {
        let mut buf = PathBuf::new();
        for segment in val.split('/') {
            if segment == ".." {
                buf.pop();
            } else if segment.starts_with('.') {
                return Err(UriSegmentError::BadStart('.'));
            } else if segment.starts_with('*') {
                return Err(UriSegmentError::BadStart('*'));
            } else if segment.ends_with(':') {
                return Err(UriSegmentError::BadEnd(':'));
            } else if segment.ends_with('>') {
                return Err(UriSegmentError::BadEnd('>'));
            } else if segment.ends_with('<') {
                return Err(UriSegmentError::BadEnd('<'));
            } else if segment.is_empty() {
                continue;
            } else if cfg!(windows) && segment.contains('\\') {
                return Err(UriSegmentError::BadChar('\\'));
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
            type Err = InternalError<<$type as FromStr>::Err>;

            fn from_param(val: &str) -> Result<Self, Self::Err> {
                <$type as FromStr>::from_str(val)
                    .map_err(|e| InternalError::new(e, StatusCode::BAD_REQUEST))
            }
        }
    };
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
        assert_eq!(
            PathBuf::from_param("/test/.tt"),
            Err(UriSegmentError::BadStart('.'))
        );
        assert_eq!(
            PathBuf::from_param("/test/*tt"),
            Err(UriSegmentError::BadStart('*'))
        );
        assert_eq!(
            PathBuf::from_param("/test/tt:"),
            Err(UriSegmentError::BadEnd(':'))
        );
        assert_eq!(
            PathBuf::from_param("/test/tt<"),
            Err(UriSegmentError::BadEnd('<'))
        );
        assert_eq!(
            PathBuf::from_param("/test/tt>"),
            Err(UriSegmentError::BadEnd('>'))
        );
        assert_eq!(
            PathBuf::from_param("/seg1/seg2/"),
            Ok(PathBuf::from_iter(vec!["seg1", "seg2"]))
        );
        assert_eq!(
            PathBuf::from_param("/seg1/../seg2/"),
            Ok(PathBuf::from_iter(vec!["seg2"]))
        );
    }
}
