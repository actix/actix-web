use http::uri::Uri;
use http::HttpTryFrom;

use super::error::InvalidUrl;
use super::pool::Key;

#[derive(Debug)]
/// `Connect` type represents a message that can be sent to
/// `Connector` with a connection request.
pub struct Connect {
    pub(crate) uri: Uri,
}

impl Connect {
    /// Create `Connect` message for specified `Uri`
    pub fn new(uri: Uri) -> Connect {
        Connect { uri }
    }

    /// Construct `Uri` instance and create `Connect` message.
    pub fn try_from<U>(uri: U) -> Result<Connect, InvalidUrl>
    where
        Uri: HttpTryFrom<U>,
    {
        Ok(Connect {
            uri: Uri::try_from(uri).map_err(|e| e.into())?,
        })
    }

    pub(crate) fn is_secure(&self) -> bool {
        if let Some(scheme) = self.uri.scheme_part() {
            scheme.as_str() == "https"
        } else {
            false
        }
    }

    pub(crate) fn key(&self) -> Key {
        self.uri.authority_part().unwrap().clone().into()
    }

    pub(crate) fn validate(&self) -> Result<(), InvalidUrl> {
        if self.uri.host().is_none() {
            Err(InvalidUrl::MissingHost)
        } else if self.uri.scheme_part().is_none() {
            Err(InvalidUrl::MissingScheme)
        } else if let Some(scheme) = self.uri.scheme_part() {
            match scheme.as_str() {
                "http" | "ws" | "https" | "wss" => Ok(()),
                _ => Err(InvalidUrl::UnknownScheme),
            }
        } else {
            Ok(())
        }
    }

    pub(crate) fn host(&self) -> &str {
        &self.uri.host().unwrap()
    }

    pub(crate) fn port(&self) -> u16 {
        if let Some(port) = self.uri.port() {
            port
        } else if let Some(scheme) = self.uri.scheme_part() {
            match scheme.as_str() {
                "http" | "ws" => 80,
                "https" | "wss" => 443,
                _ => 80,
            }
        } else {
            80
        }
    }
}
