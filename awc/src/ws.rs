//! Websockets client
//!
//! Type definitions required to use [`awc::Client`](../struct.Client.html) as a WebSocket client.
//!
//! # Example
//!
//! ```
//! use awc::{Client, ws};
//! use futures_util::{sink::SinkExt, stream::StreamExt};
//!
//! #[actix_rt::main]
//! async fn main() {
//!     let (_resp, mut connection) = Client::new()
//!         .ws("ws://echo.websocket.org")
//!         .connect()
//!         .await
//!         .unwrap();
//!
//!     connection
//!         .send(ws::Message::Text("Echo".to_string()))
//!         .await
//!         .unwrap();
//!     let response = connection.next().await.unwrap().unwrap();
//!
//!     assert_eq!(response, ws::Frame::Text("Echo".as_bytes().into()));
//! }
//! ```

use std::convert::TryFrom;
use std::net::SocketAddr;
use std::rc::Rc;
use std::{fmt, str};

use actix_codec::Framed;
use actix_http::cookie::{Cookie, CookieJar};
use actix_http::{ws, Payload, RequestHead};
use actix_rt::time::timeout;

pub use actix_http::ws::{CloseCode, CloseReason, Codec, Frame, Message};

use crate::connect::BoxedSocket;
use crate::error::{InvalidUrl, SendRequestError, WsClientError};
use crate::http::header::{
    self, HeaderName, HeaderValue, IntoHeaderValue, AUTHORIZATION,
};
use crate::http::{
    ConnectionType, Error as HttpError, Method, StatusCode, Uri, Version,
};
use crate::response::ClientResponse;
use crate::ClientConfig;

/// `WebSocket` connection
pub struct WebsocketsRequest {
    pub(crate) head: RequestHead,
    err: Option<HttpError>,
    origin: Option<HeaderValue>,
    protocols: Option<String>,
    addr: Option<SocketAddr>,
    max_size: usize,
    server_mode: bool,
    cookies: Option<CookieJar>,
    config: Rc<ClientConfig>,
}

impl WebsocketsRequest {
    /// Create new websocket connection
    pub(crate) fn new<U>(uri: U, config: Rc<ClientConfig>) -> Self
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        let mut err = None;
        let mut head = RequestHead::default();
        head.method = Method::GET;
        head.version = Version::HTTP_11;

        match Uri::try_from(uri) {
            Ok(uri) => head.uri = uri,
            Err(e) => err = Some(e.into()),
        }

        WebsocketsRequest {
            head,
            err,
            config,
            addr: None,
            origin: None,
            protocols: None,
            max_size: 65_536,
            server_mode: false,
            cookies: None,
        }
    }

    /// Set socket address of the server.
    ///
    /// This address is used for connection. If address is not
    /// provided url's host name get resolved.
    pub fn address(mut self, addr: SocketAddr) -> Self {
        self.addr = Some(addr);
        self
    }

    /// Set supported websocket protocols
    pub fn protocols<U, V>(mut self, protos: U) -> Self
    where
        U: IntoIterator<Item = V>,
        V: AsRef<str>,
    {
        let mut protos = protos
            .into_iter()
            .fold(String::new(), |acc, s| acc + s.as_ref() + ",");
        protos.pop();
        self.protocols = Some(protos);
        self
    }

    /// Set a cookie
    pub fn cookie(mut self, cookie: Cookie<'_>) -> Self {
        if self.cookies.is_none() {
            let mut jar = CookieJar::new();
            jar.add(cookie.into_owned());
            self.cookies = Some(jar)
        } else {
            self.cookies.as_mut().unwrap().add(cookie.into_owned());
        }
        self
    }

    /// Set request Origin
    pub fn origin<V, E>(mut self, origin: V) -> Self
    where
        HeaderValue: TryFrom<V, Error = E>,
        HttpError: From<E>,
    {
        match HeaderValue::try_from(origin) {
            Ok(value) => self.origin = Some(value),
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Set max frame size
    ///
    /// By default max size is set to 64kb
    pub fn max_frame_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    /// Disable payload masking. By default ws client masks frame payload.
    pub fn server_mode(mut self) -> Self {
        self.server_mode = true;
        self
    }

    /// Append a header.
    ///
    /// Header gets appended to existing header.
    /// To override header use `set_header()` method.
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match value.try_into() {
                Ok(value) => {
                    self.head.headers.append(key, value);
                }
                Err(e) => self.err = Some(e.into()),
            },
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Insert a header, replaces existing header.
    pub fn set_header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match value.try_into() {
                Ok(value) => {
                    self.head.headers.insert(key, value);
                }
                Err(e) => self.err = Some(e.into()),
            },
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Insert a header only if it is not yet set.
    pub fn set_header_if_none<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        match HeaderName::try_from(key) {
            Ok(key) => {
                if !self.head.headers.contains_key(&key) {
                    match value.try_into() {
                        Ok(value) => {
                            self.head.headers.insert(key, value);
                        }
                        Err(e) => self.err = Some(e.into()),
                    }
                }
            }
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Set HTTP basic authorization header
    pub fn basic_auth<U>(self, username: U, password: Option<&str>) -> Self
    where
        U: fmt::Display,
    {
        let auth = match password {
            Some(password) => format!("{}:{}", username, password),
            None => format!("{}:", username),
        };
        self.header(AUTHORIZATION, format!("Basic {}", base64::encode(&auth)))
    }

    /// Set HTTP bearer authentication header
    pub fn bearer_auth<T>(self, token: T) -> Self
    where
        T: fmt::Display,
    {
        self.header(AUTHORIZATION, format!("Bearer {}", token))
    }

    /// Complete request construction and connect to a websockets server.
    pub async fn connect(
        mut self,
    ) -> Result<(ClientResponse, Framed<BoxedSocket, Codec>), WsClientError> {
        if let Some(e) = self.err.take() {
            return Err(e.into());
        }

        // validate uri
        let uri = &self.head.uri;
        if uri.host().is_none() {
            return Err(InvalidUrl::MissingHost.into());
        } else if uri.scheme().is_none() {
            return Err(InvalidUrl::MissingScheme.into());
        } else if let Some(scheme) = uri.scheme() {
            match scheme.as_str() {
                "http" | "ws" | "https" | "wss" => (),
                _ => return Err(InvalidUrl::UnknownScheme.into()),
            }
        } else {
            return Err(InvalidUrl::UnknownScheme.into());
        }

        if !self.head.headers.contains_key(header::HOST) {
            self.head.headers.insert(
                header::HOST,
                HeaderValue::from_str(uri.host().unwrap()).unwrap(),
            );
        }

        // set cookies
        if let Some(ref mut jar) = self.cookies {
            let cookie: String = jar
                .delta()
                // ensure only name=value is written to cookie header
                .map(|c| Cookie::new(c.name(), c.value()).encoded().to_string())
                .collect::<Vec<_>>()
                .join("; ");

            if !cookie.is_empty() {
                self.head
                    .headers
                    .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
            }
        }

        // origin
        if let Some(origin) = self.origin.take() {
            self.head.headers.insert(header::ORIGIN, origin);
        }

        self.head.set_connection_type(ConnectionType::Upgrade);
        self.head
            .headers
            .insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        self.head.headers.insert(
            header::SEC_WEBSOCKET_VERSION,
            HeaderValue::from_static("13"),
        );

        if let Some(protocols) = self.protocols.take() {
            self.head.headers.insert(
                header::SEC_WEBSOCKET_PROTOCOL,
                HeaderValue::try_from(protocols.as_str()).unwrap(),
            );
        }

        // Generate a random key for the `Sec-WebSocket-Key` header.
        // a base64-encoded (see Section 4 of [RFC4648]) value that,
        // when decoded, is 16 bytes in length (RFC 6455)
        let sec_key: [u8; 16] = rand::random();
        let key = base64::encode(&sec_key);

        self.head.headers.insert(
            header::SEC_WEBSOCKET_KEY,
            HeaderValue::try_from(key.as_str()).unwrap(),
        );

        let head = self.head;
        let max_size = self.max_size;
        let server_mode = self.server_mode;

        let fut = self
            .config
            .connector
            .borrow_mut()
            .open_tunnel(head, self.addr);

        // set request timeout
        let (head, framed) = if let Some(to) = self.config.timeout {
            timeout(to, fut)
                .await
                .map_err(|_| SendRequestError::Timeout)
                .and_then(|res| res)?
        } else {
            fut.await?
        };

        // verify response
        if head.status != StatusCode::SWITCHING_PROTOCOLS {
            return Err(WsClientError::InvalidResponseStatus(head.status));
        }

        // Check for "UPGRADE" to websocket header
        let has_hdr = if let Some(hdr) = head.headers.get(&header::UPGRADE) {
            if let Ok(s) = hdr.to_str() {
                s.to_ascii_lowercase().contains("websocket")
            } else {
                false
            }
        } else {
            false
        };
        if !has_hdr {
            log::trace!("Invalid upgrade header");
            return Err(WsClientError::InvalidUpgradeHeader);
        }

        // Check for "CONNECTION" header
        if let Some(conn) = head.headers.get(&header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                if !s.to_ascii_lowercase().contains("upgrade") {
                    log::trace!("Invalid connection header: {}", s);
                    return Err(WsClientError::InvalidConnectionHeader(conn.clone()));
                }
            } else {
                log::trace!("Invalid connection header: {:?}", conn);
                return Err(WsClientError::InvalidConnectionHeader(conn.clone()));
            }
        } else {
            log::trace!("Missing connection header");
            return Err(WsClientError::MissingConnectionHeader);
        }

        if let Some(hdr_key) = head.headers.get(&header::SEC_WEBSOCKET_ACCEPT) {
            let encoded = ws::hash_key(key.as_ref());
            if hdr_key.as_bytes() != encoded.as_bytes() {
                log::trace!(
                    "Invalid challenge response: expected: {} received: {:?}",
                    encoded,
                    key
                );
                return Err(WsClientError::InvalidChallengeResponse(
                    encoded,
                    hdr_key.clone(),
                ));
            }
        } else {
            log::trace!("Missing SEC-WEBSOCKET-ACCEPT header");
            return Err(WsClientError::MissingWebSocketAcceptHeader);
        };

        // response and ws framed
        Ok((
            ClientResponse::new(head, Payload::None),
            framed.into_map_codec(|_| {
                if server_mode {
                    ws::Codec::new().max_size(max_size)
                } else {
                    ws::Codec::new().max_size(max_size).client_mode()
                }
            }),
        ))
    }
}

impl fmt::Debug for WebsocketsRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "\nWebsocketsRequest {}:{}",
            self.head.method, self.head.uri
        )?;
        writeln!(f, "  headers:")?;
        for (key, val) in self.head.headers.iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Client;

    #[actix_rt::test]
    async fn test_debug() {
        let request = Client::new().ws("/").header("x-test", "111");
        let repr = format!("{:?}", request);
        assert!(repr.contains("WebsocketsRequest"));
        assert!(repr.contains("x-test"));
    }

    #[actix_rt::test]
    async fn test_header_override() {
        let req = Client::builder()
            .header(header::CONTENT_TYPE, "111")
            .finish()
            .ws("/")
            .set_header(header::CONTENT_TYPE, "222");

        assert_eq!(
            req.head
                .headers
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            "222"
        );
    }

    #[actix_rt::test]
    async fn basic_auth() {
        let req = Client::new()
            .ws("/")
            .basic_auth("username", Some("password"));
        assert_eq!(
            req.head
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Basic dXNlcm5hbWU6cGFzc3dvcmQ="
        );

        let req = Client::new().ws("/").basic_auth("username", None);
        assert_eq!(
            req.head
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Basic dXNlcm5hbWU6"
        );
    }

    #[actix_rt::test]
    async fn bearer_auth() {
        let req = Client::new().ws("/").bearer_auth("someS3cr3tAutht0k3n");
        assert_eq!(
            req.head
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer someS3cr3tAutht0k3n"
        );
        let _ = req.connect();
    }

    #[actix_rt::test]
    async fn basics() {
        let req = Client::new()
            .ws("http://localhost/")
            .origin("test-origin")
            .max_frame_size(100)
            .server_mode()
            .protocols(&["v1", "v2"])
            .set_header_if_none(header::CONTENT_TYPE, "json")
            .set_header_if_none(header::CONTENT_TYPE, "text")
            .cookie(Cookie::build("cookie1", "value1").finish());
        assert_eq!(
            req.origin.as_ref().unwrap().to_str().unwrap(),
            "test-origin"
        );
        assert_eq!(req.max_size, 100);
        assert_eq!(req.server_mode, true);
        assert_eq!(req.protocols, Some("v1,v2".to_string()));
        assert_eq!(
            req.head.headers.get(header::CONTENT_TYPE).unwrap(),
            header::HeaderValue::from_static("json")
        );

        let _ = req.connect().await;

        assert!(Client::new().ws("/").connect().await.is_err());
        assert!(Client::new().ws("http:///test").connect().await.is_err());
        assert!(Client::new().ws("hmm://test.com/").connect().await.is_err());
    }
}
