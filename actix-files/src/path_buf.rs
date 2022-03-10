use std::{
    path::{Component, Path, PathBuf},
    str::FromStr,
};

use actix_utils::future::{ready, Ready};
use actix_web::{dev::Payload, FromRequest, HttpRequest};

use crate::error::UriSegmentError;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PathBufWrap(PathBuf);

impl FromStr for PathBufWrap {
    type Err = UriSegmentError;

    fn from_str(path: &str) -> Result<Self, Self::Err> {
        Self::parse_path(path, false)
    }
}

impl PathBufWrap {
    /// Parse a path, giving the choice of allowing hidden files to be considered valid segments.
    ///
    /// Path traversal is guarded by this method.
    pub fn parse_path(path: &str, hidden_files: bool) -> Result<Self, UriSegmentError> {
        let mut buf = PathBuf::new();

        // equivalent to `path.split('/').count()`
        let mut segment_count = path.matches('/').count() + 1;

        // we can decode the whole path here (instead of per-segment decoding)
        // because we will reject `%2F` in paths using `segement_count`.
        let path = percent_encoding::percent_decode_str(path)
            .decode_utf8()
            .map_err(|_| UriSegmentError::NotValidUtf8)?;

        // disallow decoding `%2F` into `/`
        if segment_count != path.matches('/').count() + 1 {
            return Err(UriSegmentError::BadChar('/'));
        }

        for segment in path.split('/') {
            if segment == ".." {
                segment_count -= 1;
                buf.pop();
            } else if !hidden_files && segment.starts_with('.') {
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
                segment_count -= 1;
                continue;
            } else if cfg!(windows) && segment.contains('\\') {
                return Err(UriSegmentError::BadChar('\\'));
            } else if cfg!(windows) && segment.contains(':') {
                return Err(UriSegmentError::BadChar(':'));
            } else {
                buf.push(segment)
            }
        }

        // make sure we agree with stdlib parser
        for (i, component) in buf.components().enumerate() {
            assert!(
                matches!(component, Component::Normal(_)),
                "component `{:?}` is not normal",
                component
            );
            assert!(i < segment_count);
        }

        Ok(PathBufWrap(buf))
    }
}

impl AsRef<Path> for PathBufWrap {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

impl FromRequest for PathBufWrap {
    type Error = UriSegmentError;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ready(req.match_info().unprocessed().parse())
    }
}

#[cfg(test)]
mod tests {
    use std::iter::FromIterator;

    use super::*;

    #[test]
    fn test_path_buf() {
        assert_eq!(
            PathBufWrap::from_str("/test/.tt").map(|t| t.0),
            Err(UriSegmentError::BadStart('.'))
        );
        assert_eq!(
            PathBufWrap::from_str("/test/*tt").map(|t| t.0),
            Err(UriSegmentError::BadStart('*'))
        );
        assert_eq!(
            PathBufWrap::from_str("/test/tt:").map(|t| t.0),
            Err(UriSegmentError::BadEnd(':'))
        );
        assert_eq!(
            PathBufWrap::from_str("/test/tt<").map(|t| t.0),
            Err(UriSegmentError::BadEnd('<'))
        );
        assert_eq!(
            PathBufWrap::from_str("/test/tt>").map(|t| t.0),
            Err(UriSegmentError::BadEnd('>'))
        );
        assert_eq!(
            PathBufWrap::from_str("/seg1/seg2/").unwrap().0,
            PathBuf::from_iter(vec!["seg1", "seg2"])
        );
        assert_eq!(
            PathBufWrap::from_str("/seg1/../seg2/").unwrap().0,
            PathBuf::from_iter(vec!["seg2"])
        );
    }

    #[test]
    fn test_parse_path() {
        assert_eq!(
            PathBufWrap::parse_path("/test/.tt", false).map(|t| t.0),
            Err(UriSegmentError::BadStart('.'))
        );

        assert_eq!(
            PathBufWrap::parse_path("/test/.tt", true).unwrap().0,
            PathBuf::from_iter(vec!["test", ".tt"])
        );
    }

    #[test]
    fn path_traversal() {
        assert_eq!(
            PathBufWrap::parse_path("/../README.md", false).unwrap().0,
            PathBuf::from_iter(vec!["README.md"])
        );

        assert_eq!(
            PathBufWrap::parse_path("/../README.md", true).unwrap().0,
            PathBuf::from_iter(vec!["README.md"])
        );

        assert_eq!(
            PathBufWrap::parse_path("/../../../../../../../../../../etc/passwd", false)
                .unwrap()
                .0,
            PathBuf::from_iter(vec!["etc/passwd"])
        );
    }

    #[test]
    #[cfg_attr(windows, should_panic)]
    fn windows_drive_traversal() {
        // detect issues in windows that could lead to path traversal
        // see <https://github.com/SergioBenitez/Rocket/issues/1949

        assert_eq!(
            PathBufWrap::parse_path("C:test.txt", false).unwrap().0,
            PathBuf::from_iter(vec!["C:test.txt"])
        );

        assert_eq!(
            PathBufWrap::parse_path("C:../whatever", false).unwrap().0,
            PathBuf::from_iter(vec!["C:../whatever"])
        );

        assert_eq!(
            PathBufWrap::parse_path(":test.txt", false).unwrap().0,
            PathBuf::from_iter(vec![":test.txt"])
        );
    }
}
