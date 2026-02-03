use crate::{Quoter, ResourcePath};

thread_local! {
    static DEFAULT_QUOTER: Quoter = Quoter::new(b"", b"%/+");
}

#[derive(Debug, Clone, Default)]
pub struct Url {
    uri: http::Uri,
    path: Option<String>,
}

impl Url {
    #[inline]
    pub fn new(uri: http::Uri) -> Url {
        let path = DEFAULT_QUOTER.with(|q| q.requote_str_lossy(uri.path()));
        Url { uri, path }
    }

    #[inline]
    pub fn new_with_quoter(uri: http::Uri, quoter: &Quoter) -> Url {
        Url {
            path: quoter.requote_str_lossy(uri.path()),
            uri,
        }
    }

    /// Returns URI.
    #[inline]
    pub fn uri(&self) -> &http::Uri {
        &self.uri
    }

    /// Returns path.
    #[inline]
    pub fn path(&self) -> &str {
        match self.path {
            Some(ref path) => path,
            _ => self.uri.path(),
        }
    }

    #[inline]
    pub fn update(&mut self, uri: &http::Uri) {
        self.uri = uri.clone();
        self.path = DEFAULT_QUOTER.with(|q| q.requote_str_lossy(uri.path()));
    }

    #[inline]
    pub fn update_with_quoter(&mut self, uri: &http::Uri, quoter: &Quoter) {
        self.uri = uri.clone();
        self.path = quoter.requote_str_lossy(uri.path());
    }
}

impl ResourcePath for Url {
    #[inline]
    fn path(&self) -> &str {
        self.path()
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use http::Uri;

    use super::*;
    use crate::{Path, ResourceDef};

    const PROTECTED: &[u8] = b"%/+";

    fn match_url(pattern: &'static str, url: impl AsRef<str>) -> Path<Url> {
        let re = ResourceDef::new(pattern);
        let uri = Uri::try_from(url.as_ref()).unwrap();
        let mut path = Path::new(Url::new(uri));
        assert!(re.capture_match_info(&mut path));
        path
    }

    fn percent_encode(data: &[u8]) -> String {
        data.iter()
            .fold(String::with_capacity(data.len() * 3), |mut buf, c| {
                write!(&mut buf, "%{:02X}", c).unwrap();
                buf
            })
    }

    #[test]
    fn parse_url() {
        let re = "/user/{id}/test";

        let path = match_url(re, "/user/2345/test");
        assert_eq!(path.get("id").unwrap(), "2345");
    }

    #[test]
    fn protected_chars() {
        let re = "/user/{id}/test";

        let encoded = percent_encode(PROTECTED);
        let path = match_url(re, format!("/user/{}/test", encoded));
        // characters in captured segment remain unencoded
        assert_eq!(path.get("id").unwrap(), &encoded);

        // "%25" should never be decoded into '%' to guarantee the output is a valid
        // percent-encoded format
        let path = match_url(re, "/user/qwe%25/test");
        assert_eq!(path.get("id").unwrap(), "qwe%25");

        let path = match_url(re, "/user/qwe%25rty/test");
        assert_eq!(path.get("id").unwrap(), "qwe%25rty");
    }

    #[test]
    fn non_protected_ascii() {
        let non_protected_ascii = ('\u{0}'..='\u{7F}')
            .filter(|&c| c.is_ascii() && !PROTECTED.contains(&(c as u8)))
            .collect::<String>();
        let encoded = percent_encode(non_protected_ascii.as_bytes());
        let path = match_url("/user/{id}/test", format!("/user/{}/test", encoded));
        assert_eq!(path.get("id").unwrap(), &non_protected_ascii);
    }

    #[test]
    fn valid_utf8_multi_byte() {
        let test = ('\u{FF00}'..='\u{FFFF}').collect::<String>();
        let encoded = percent_encode(test.as_bytes());
        let path = match_url("/a/{id}/b", format!("/a/{}/b", &encoded));
        assert_eq!(path.get("id").unwrap(), &test);
    }

    #[test]
    fn invalid_utf8() {
        let invalid_utf8 = percent_encode((0x80..=0xff).collect::<Vec<_>>().as_slice());
        let uri = Uri::try_from(format!("/{}", invalid_utf8)).unwrap();
        let path = Path::new(Url::new(uri));

        // We should always get a valid utf8 string
        assert!(String::from_utf8(path.as_str().as_bytes().to_owned()).is_ok());
    }
}
