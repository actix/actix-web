//! Websockets client
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::rc::Rc;
use std::{fmt, str};

use actix_codec::Framed;
use actix_http::cookie::{Cookie, CookieJar};
use actix_http::{ws, Payload, RequestHead};
use bytes::{BufMut, BytesMut};
use futures::future::{err, Either, Future};
use percent_encoding::{percent_encode, USERINFO_ENCODE_SET};
use tokio_timer::Timeout;

pub use actix_http::ws::{CloseCode, CloseReason, Codec, Frame, Message};

use crate::connect::BoxedSocket;
use crate::error::{InvalidUrl, SendRequestError, WsClientError};
use crate::http::header::{
    self, HeaderName, HeaderValue, IntoHeaderValue, AUTHORIZATION,
};
use crate::http::{
    ConnectionType, Error as HttpError, HttpTryFrom, Method, StatusCode, Uri, Version,
};
use crate::response::ClientResponse;
use crate::ClientConfig;

/// `WebSocket` connection
pub struct WebsocketsRequest {
    pub(crate) head: RequestHead,
    err: Option<HttpError>,
    origin: Option<HeaderValue>,
    protocols: Option<String>,
    max_size: usize,
    server_mode: bool,
    default_headers: bool,
    cookies: Option<CookieJar>,
    config: Rc<ClientConfig>,
}

impl WebsocketsRequest {
    /// Create new websocket connection
    pub(crate) fn new<U>(uri: U, config: Rc<ClientConfig>) -> Self
    where
        Uri: HttpTryFrom<U>,
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
            origin: None,
            protocols: None,
            max_size: 65_536,
            server_mode: false,
            cookies: None,
            default_headers: true,
        }
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
    pub fn cookie<'c>(mut self, cookie: Cookie<'c>) -> Self {
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
    pub fn origin<V>(mut self, origin: V) -> Self
    where
        HeaderValue: HttpTryFrom<V>,
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

    /// Do not add default request headers.
    /// By default `Date` and `User-Agent` headers are set.
    pub fn no_default_headers(mut self) -> Self {
        self.default_headers = false;
        self
    }

    /// Append a header.
    ///
    /// Header gets appended to existing header.
    /// To override header use `set_header()` method.
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
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
        HeaderName: HttpTryFrom<K>,
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
        HeaderName: HttpTryFrom<K>,
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
    pub fn basic_auth<U, P>(self, username: U, password: Option<P>) -> Self
    where
        U: fmt::Display,
        P: fmt::Display,
    {
        let auth = match password {
            Some(password) => format!("{}:{}", username, password),
            None => format!("{}", username),
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
    pub fn connect(
        mut self,
    ) -> impl Future<Item = (ClientResponse, Framed<BoxedSocket, Codec>), Error = WsClientError>
    {
        if let Some(e) = self.err.take() {
            return Either::A(err(e.into()));
        }

        // validate uri
        let uri = &self.head.uri;
        if uri.host().is_none() {
            return Either::A(err(InvalidUrl::MissingHost.into()));
        } else if uri.scheme_part().is_none() {
            return Either::A(err(InvalidUrl::MissingScheme.into()));
        } else if let Some(scheme) = uri.scheme_part() {
            match scheme.as_str() {
                "http" | "ws" | "https" | "wss" => (),
                _ => return Either::A(err(InvalidUrl::UnknownScheme.into())),
            }
        } else {
            return Either::A(err(InvalidUrl::UnknownScheme.into()));
        }

        // set default headers
        let mut slf = if self.default_headers {
            // set request host header
            if let Some(host) = self.head.uri.host() {
                if !self.head.headers.contains_key(header::HOST) {
                    let mut wrt = BytesMut::with_capacity(host.len() + 5).writer();

                    let _ = match self.head.uri.port_u16() {
                        None | Some(80) | Some(443) => write!(wrt, "{}", host),
                        Some(port) => write!(wrt, "{}:{}", host, port),
                    };

                    match wrt.get_mut().take().freeze().try_into() {
                        Ok(value) => {
                            self.head.headers.insert(header::HOST, value);
                        }
                        Err(e) => return Either::A(err(HttpError::from(e).into())),
                    }
                }
            }

            // user agent
            self.set_header_if_none(
                header::USER_AGENT,
                concat!("awc/", env!("CARGO_PKG_VERSION")),
            )
        } else {
            self
        };

        let mut head = slf.head;

        // set cookies
        if let Some(ref mut jar) = slf.cookies {
            let mut cookie = String::new();
            for c in jar.delta() {
                let name = percent_encode(c.name().as_bytes(), USERINFO_ENCODE_SET);
                let value = percent_encode(c.value().as_bytes(), USERINFO_ENCODE_SET);
                let _ = write!(&mut cookie, "; {}={}", name, value);
            }
            head.headers.insert(
                header::COOKIE,
                HeaderValue::from_str(&cookie.as_str()[2..]).unwrap(),
            );
        }

        // origin
        if let Some(origin) = slf.origin.take() {
            head.headers.insert(header::ORIGIN, origin);
        }

        head.set_connection_type(ConnectionType::Upgrade);
        head.headers
            .insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        head.headers.insert(
            header::SEC_WEBSOCKET_VERSION,
            HeaderValue::from_static("13"),
        );

        if let Some(protocols) = slf.protocols.take() {
            head.headers.insert(
                header::SEC_WEBSOCKET_PROTOCOL,
                HeaderValue::try_from(protocols.as_str()).unwrap(),
            );
        }

        // Generate a random key for the `Sec-WebSocket-Key` header.
        // a base64-encoded (see Section 4 of [RFC4648]) value that,
        // when decoded, is 16 bytes in length (RFC 6455)
        let sec_key: [u8; 16] = rand::random();
        let key = base64::encode(&sec_key);

        head.headers.insert(
            header::SEC_WEBSOCKET_KEY,
            HeaderValue::try_from(key.as_str()).unwrap(),
        );

        let max_size = slf.max_size;
        let server_mode = slf.server_mode;

        let fut = slf
            .config
            .connector
            .borrow_mut()
            .open_tunnel(head)
            .from_err()
            .and_then(move |(head, framed)| {
                // verify response
                if head.status != StatusCode::SWITCHING_PROTOCOLS {
                    return Err(WsClientError::InvalidResponseStatus(head.status));
                }
                // Check for "UPGRADE" to websocket header
                let has_hdr = if let Some(hdr) = head.headers.get(header::UPGRADE) {
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
                if let Some(conn) = head.headers.get(header::CONNECTION) {
                    if let Ok(s) = conn.to_str() {
                        if !s.to_ascii_lowercase().contains("upgrade") {
                            log::trace!("Invalid connection header: {}", s);
                            return Err(WsClientError::InvalidConnectionHeader(
                                conn.clone(),
                            ));
                        }
                    } else {
                        log::trace!("Invalid connection header: {:?}", conn);
                        return Err(WsClientError::InvalidConnectionHeader(conn.clone()));
                    }
                } else {
                    log::trace!("Missing connection header");
                    return Err(WsClientError::MissingConnectionHeader);
                }

                if let Some(hdr_key) = head.headers.get(header::SEC_WEBSOCKET_ACCEPT) {
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
                    framed.map_codec(|_| {
                        if server_mode {
                            ws::Codec::new().max_size(max_size)
                        } else {
                            ws::Codec::new().max_size(max_size).client_mode()
                        }
                    }),
                ))
            });

        // set request timeout
        if let Some(timeout) = slf.config.timeout {
            Either::B(Either::A(Timeout::new(fut, timeout).map_err(|e| {
                if let Some(e) = e.into_inner() {
                    e
                } else {
                    SendRequestError::Timeout.into()
                }
            })))
        } else {
            Either::B(Either::B(fut))
        }
    }
}
