use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use actix_web::{dev::Payload, FromRequest, HttpRequest};
use futures_util::future::{ready, Ready};

use crate::error::UriSegmentError;

#[derive(Debug)]
pub(crate) struct PathBufWrap(PathBuf);

impl FromStr for PathBufWrap {
    type Err = UriSegmentError;

    fn from_str(path: &str) -> Result<Self, Self::Err> {
        Self::parse_path(path, false)
    }
}

impl PathBufWrap {
    /// Parse a path, giving the choice of allowing hidden files to be considered valid segments.
    /// If the path contains `".."`, an error will be returned to prevent someone being able
    /// to traverse the filesystem.
    pub fn parse_path(path: &str, hidden_files: bool) -> Result<Self, UriSegmentError> {
        let mut buf = PathBuf::new();

        for segment in path.split('/') {
            if !hidden_files && segment.starts_with('.') {
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
            } else if segment.contains("..") {
                return Err(UriSegmentError::BadStart('.'));
            } else {
                buf.push(segment)
            }
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
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ready(req.match_info().path().parse())
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
            PathBufWrap::from_str("/seg1/../seg2/").map(|t| t.0),
            Err(UriSegmentError::BadStart('.'))
        );
        assert_eq!(
            PathBufWrap::from_str("/seg1/..%5c/..%5c/seg2/test.txt").map(|t| t.0),
            Err(UriSegmentError::BadStart('.'))
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

        assert_eq!(
            PathBufWrap::parse_path("/seg1/..%5c/..%5c/seg2/test.txt", true).map(|t| t.0),
            Err(UriSegmentError::BadStart('.'))
        );
    }
}
