use std::{fmt, net, rc::Rc, time::Duration};

use actix_http::{
    body::MessageBody,
    error::HttpError,
    header::{self, HeaderMap, HeaderValue, TryIntoHeaderPair},
    ConnectionType, Method, RequestHead, Uri, Version,
};
use base64::prelude::*;
use bytes::Bytes;
use futures_core::Stream;
use serde::Serialize;

#[cfg(feature = "cookies")]
use crate::cookie::{Cookie, CookieJar};
use crate::{
    client::ClientConfig,
    error::{FreezeRequestError, InvalidUrl},
    frozen::FrozenClientRequest,
    sender::{PrepForSendingError, RequestSender, SendClientRequest},
    BoxError,
};

/// An HTTP Client request builder
///
/// This type can be used to construct an instance of `ClientRequest` through a
/// builder-like pattern.
///
/// ```no_run
/// # #[actix_rt::main]
/// # async fn main() {
/// let response = awc::Client::new()
///      .get("http://www.rust-lang.org") // <- Create request builder
///      .insert_header(("User-Agent", "Actix-web"))
///      .send()                          // <- Send HTTP request
///      .await;
///
/// response.and_then(|response| {   // <- server HTTP response
///      println!("Response: {:?}", response);
///      Ok(())
/// });
/// # }
/// ```
pub struct ClientRequest {
    pub(crate) head: RequestHead,
    err: Option<HttpError>,
    addr: Option<net::SocketAddr>,
    response_decompress: bool,
    timeout: Option<Duration>,
    config: ClientConfig,

    #[cfg(feature = "cookies")]
    cookies: Option<CookieJar>,
}

impl ClientRequest {
    /// Create new client request builder.
    pub(crate) fn new<U>(method: Method, uri: U, config: ClientConfig) -> Self
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        ClientRequest {
            config,
            head: RequestHead::default(),
            err: None,
            addr: None,
            #[cfg(feature = "cookies")]
            cookies: None,
            timeout: None,
            response_decompress: true,
        }
        .method(method)
        .uri(uri)
    }

    /// Set HTTP URI of request.
    #[inline]
    pub fn uri<U>(mut self, uri: U) -> Self
    where
        Uri: TryFrom<U>,
        <Uri as TryFrom<U>>::Error: Into<HttpError>,
    {
        match Uri::try_from(uri) {
            Ok(uri) => self.head.uri = uri,
            Err(err) => self.err = Some(err.into()),
        }
        self
    }

    /// Get HTTP URI of request.
    pub fn get_uri(&self) -> &Uri {
        &self.head.uri
    }

    /// Set socket address of the server.
    ///
    /// This address is used for connection. If address is not
    /// provided url's host name get resolved.
    pub fn address(mut self, addr: net::SocketAddr) -> Self {
        self.addr = Some(addr);
        self
    }

    /// Set HTTP method of this request.
    #[inline]
    pub fn method(mut self, method: Method) -> Self {
        self.head.method = method;
        self
    }

    /// Get HTTP method of this request
    pub fn get_method(&self) -> &Method {
        &self.head.method
    }

    /// Set HTTP version of this request.
    ///
    /// By default requests's HTTP version depends on network stream
    #[doc(hidden)]
    #[inline]
    pub fn version(mut self, version: Version) -> Self {
        self.head.version = version;
        self
    }

    /// Get HTTP version of this request.
    pub fn get_version(&self) -> &Version {
        &self.head.version
    }

    /// Get peer address of this request.
    pub fn get_peer_addr(&self) -> &Option<net::SocketAddr> {
        &self.head.peer_addr
    }

    /// Returns request's headers.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    /// Returns request's mutable headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.head.headers
    }

    /// Insert a header, replacing any that were set with an equivalent field name.
    pub fn insert_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        match header.try_into_pair() {
            Ok((key, value)) => {
                self.head.headers.insert(key, value);
            }
            Err(err) => self.err = Some(err.into()),
        };

        self
    }

    /// Insert a header only if it is not yet set.
    pub fn insert_header_if_none(mut self, header: impl TryIntoHeaderPair) -> Self {
        match header.try_into_pair() {
            Ok((key, value)) => {
                if !self.head.headers.contains_key(&key) {
                    self.head.headers.insert(key, value);
                }
            }
            Err(err) => self.err = Some(err.into()),
        };

        self
    }

    /// Append a header, keeping any that were set with an equivalent field name.
    ///
    /// ```no_run
    /// use awc::{http::header, Client};
    ///
    /// Client::new()
    ///     .get("http://www.rust-lang.org")
    ///     .insert_header(("X-TEST", "value"))
    ///     .insert_header((header::CONTENT_TYPE, mime::APPLICATION_JSON));
    /// ```
    pub fn append_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        match header.try_into_pair() {
            Ok((key, value)) => self.head.headers.append(key, value),
            Err(err) => self.err = Some(err.into()),
        };

        self
    }

    /// Send headers in `Camel-Case` form.
    #[inline]
    pub fn camel_case(mut self) -> Self {
        self.head.set_camel_case_headers(true);
        self
    }

    /// Force close connection instead of returning it back to connections pool.
    /// This setting affect only HTTP/1 connections.
    #[inline]
    pub fn force_close(mut self) -> Self {
        self.head.set_connection_type(ConnectionType::Close);
        self
    }

    /// Set request's content type
    #[inline]
    pub fn content_type<V>(mut self, value: V) -> Self
    where
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        match HeaderValue::try_from(value) {
            Ok(value) => {
                self.head.headers.insert(header::CONTENT_TYPE, value);
            }
            Err(err) => self.err = Some(err.into()),
        }
        self
    }

    /// Set content length
    #[inline]
    pub fn content_length(self, len: u64) -> Self {
        let mut buf = itoa::Buffer::new();
        self.insert_header((header::CONTENT_LENGTH, buf.format(len)))
    }

    /// Set HTTP basic authorization header.
    ///
    /// If no password is needed, just provide an empty string.
    pub fn basic_auth(self, username: impl fmt::Display, password: impl fmt::Display) -> Self {
        let auth = format!("{}:{}", username, password);

        self.insert_header((
            header::AUTHORIZATION,
            format!("Basic {}", BASE64_STANDARD.encode(auth)),
        ))
    }

    /// Set HTTP bearer authentication header
    pub fn bearer_auth(self, token: impl fmt::Display) -> Self {
        self.insert_header((header::AUTHORIZATION, format!("Bearer {}", token)))
    }

    /// Set a cookie
    ///
    /// ```no_run
    /// use awc::{cookie::Cookie, Client};
    ///
    /// # #[actix_rt::main]
    /// # async fn main() {
    /// let res = Client::new().get("https://httpbin.org/cookies")
    ///     .cookie(Cookie::new("name", "value"))
    ///     .send()
    ///     .await;
    ///
    /// println!("Response: {:?}", res);
    /// # }
    /// ```
    #[cfg(feature = "cookies")]
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

    /// Disable automatic decompress of response's body
    pub fn no_decompress(mut self) -> Self {
        self.response_decompress = false;
        self
    }

    /// Set request timeout. Overrides client wide timeout setting.
    ///
    /// Request timeout is the total time before a response must be received.
    /// Default value is 5 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the query part of the request
    pub fn query<T: Serialize>(mut self, query: &T) -> Result<Self, serde_urlencoded::ser::Error> {
        let mut parts = self.head.uri.clone().into_parts();

        if let Some(path_and_query) = parts.path_and_query {
            let query = serde_urlencoded::to_string(query)?;
            let path = path_and_query.path();
            parts.path_and_query = format!("{}?{}", path, query).parse().ok();

            match Uri::from_parts(parts) {
                Ok(uri) => self.head.uri = uri,
                Err(err) => self.err = Some(err.into()),
            }
        }

        Ok(self)
    }

    /// Freeze request builder and construct `FrozenClientRequest`,
    /// which could be used for sending same request multiple times.
    pub fn freeze(self) -> Result<FrozenClientRequest, FreezeRequestError> {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(err) => return Err(err.into()),
        };

        let request = FrozenClientRequest {
            head: Rc::new(slf.head),
            addr: slf.addr,
            response_decompress: slf.response_decompress,
            timeout: slf.timeout,
            config: slf.config,
        };

        Ok(request)
    }

    /// Complete request construction and send body.
    pub fn send_body<B>(self, body: B) -> SendClientRequest
    where
        B: MessageBody + 'static,
    {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(err) => return err.into(),
        };

        RequestSender::Owned(slf.head).send_body(
            slf.addr,
            slf.response_decompress,
            slf.timeout,
            &slf.config,
            body,
        )
    }

    /// Set a JSON body and generate `ClientRequest`
    pub fn send_json<T: Serialize>(self, value: &T) -> SendClientRequest {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(err) => return err.into(),
        };

        RequestSender::Owned(slf.head).send_json(
            slf.addr,
            slf.response_decompress,
            slf.timeout,
            &slf.config,
            value,
        )
    }

    /// Set a urlencoded body and generate `ClientRequest`
    ///
    /// `ClientRequestBuilder` can not be used after this call.
    pub fn send_form<T: Serialize>(self, value: &T) -> SendClientRequest {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(err) => return err.into(),
        };

        RequestSender::Owned(slf.head).send_form(
            slf.addr,
            slf.response_decompress,
            slf.timeout,
            &slf.config,
            value,
        )
    }

    /// Set an streaming body and generate `ClientRequest`.
    pub fn send_stream<S, E>(self, stream: S) -> SendClientRequest
    where
        S: Stream<Item = Result<Bytes, E>> + 'static,
        E: Into<BoxError> + 'static,
    {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(err) => return err.into(),
        };

        RequestSender::Owned(slf.head).send_stream(
            slf.addr,
            slf.response_decompress,
            slf.timeout,
            &slf.config,
            stream,
        )
    }

    /// Set an empty body and generate `ClientRequest`.
    pub fn send(self) -> SendClientRequest {
        let slf = match self.prep_for_sending() {
            Ok(slf) => slf,
            Err(err) => return err.into(),
        };

        RequestSender::Owned(slf.head).send(
            slf.addr,
            slf.response_decompress,
            slf.timeout,
            &slf.config,
        )
    }

    // allow unused mut when cookies feature is disabled
    fn prep_for_sending(#[allow(unused_mut)] mut self) -> Result<Self, PrepForSendingError> {
        if let Some(err) = self.err {
            return Err(err.into());
        }

        // validate uri
        let uri = &self.head.uri;
        if uri.host().is_none() {
            return Err(InvalidUrl::MissingHost.into());
        } else if uri.scheme().is_none() {
            return Err(InvalidUrl::MissingScheme.into());
        } else if let Some(scheme) = uri.scheme() {
            match scheme.as_str() {
                "http" | "ws" | "https" | "wss" => {}
                _ => return Err(InvalidUrl::UnknownScheme.into()),
            }
        } else {
            return Err(InvalidUrl::UnknownScheme.into());
        }

        // set cookies
        #[cfg(feature = "cookies")]
        if let Some(ref mut jar) = self.cookies {
            let cookie: String = jar
                .delta()
                // ensure only name=value is written to cookie header
                .map(|c| c.stripped().encoded().to_string())
                .collect::<Vec<_>>()
                .join("; ");

            if !cookie.is_empty() {
                self.head
                    .headers
                    .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
            }
        }

        let mut slf = self;

        // Set Accept-Encoding HTTP header depending on enabled feature.
        // If decompress is not ask, then we are not able to find which encoding is
        // supported, so we cannot guess Accept-Encoding HTTP header.
        if slf.response_decompress {
            // Set Accept-Encoding with compression algorithm awc is built with.
            #[allow(clippy::vec_init_then_push)]
            #[cfg(feature = "__compress")]
            let accept_encoding = {
                let mut encoding = vec![];

                #[cfg(feature = "compress-brotli")]
                {
                    encoding.push("br");
                }

                #[cfg(feature = "compress-gzip")]
                {
                    encoding.push("gzip");
                    encoding.push("deflate");
                }

                #[cfg(feature = "compress-zstd")]
                encoding.push("zstd");

                assert!(
                    !encoding.is_empty(),
                    "encoding can not be empty unless __compress feature has been explicitly enabled"
                );

                encoding.join(", ")
            };

            // Otherwise tell the server, we do not support any compression algorithm.
            // So we clearly indicate that we do want identity encoding.
            #[cfg(not(feature = "__compress"))]
            let accept_encoding = "identity";

            slf = slf.insert_header_if_none((header::ACCEPT_ENCODING, accept_encoding));
        }

        Ok(slf)
    }
}

impl fmt::Debug for ClientRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "\nClientRequest {:?} {} {}",
            self.head.version, self.head.method, self.head.uri
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
    use std::time::SystemTime;

    use actix_http::header::HttpDate;

    use super::*;
    use crate::Client;

    #[actix_rt::test]
    async fn test_debug() {
        let request = Client::new().get("/").append_header(("x-test", "111"));
        let repr = format!("{:?}", request);
        assert!(repr.contains("ClientRequest"));
        assert!(repr.contains("x-test"));
    }

    #[actix_rt::test]
    async fn test_basics() {
        let req = Client::new()
            .put("/")
            .version(Version::HTTP_2)
            .insert_header((header::DATE, HttpDate::from(SystemTime::now())))
            .content_type("plain/text")
            .append_header((header::SERVER, "awc"));

        let req = if let Some(val) = Some("server") {
            req.append_header((header::USER_AGENT, val))
        } else {
            req
        };

        let req = if let Some(_val) = Option::<&str>::None {
            req.append_header((header::ALLOW, "1"))
        } else {
            req
        };

        let mut req = req.content_length(100);

        assert!(req.headers().contains_key(header::CONTENT_TYPE));
        assert!(req.headers().contains_key(header::DATE));
        assert!(req.headers().contains_key(header::SERVER));
        assert!(req.headers().contains_key(header::USER_AGENT));
        assert!(!req.headers().contains_key(header::ALLOW));
        assert!(!req.headers().contains_key(header::EXPECT));
        assert_eq!(req.head.version, Version::HTTP_2);

        let _ = req.headers_mut();

        #[allow(clippy::let_underscore_future)]
        let _ = req.send_body("");
    }

    #[actix_rt::test]
    async fn test_client_header() {
        let req = Client::builder()
            .add_default_header((header::CONTENT_TYPE, "111"))
            .finish()
            .get("/");

        assert_eq!(
            req.head
                .headers
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            "111"
        );
    }

    #[actix_rt::test]
    async fn test_client_header_override() {
        let req = Client::builder()
            .add_default_header((header::CONTENT_TYPE, "111"))
            .finish()
            .get("/")
            .insert_header((header::CONTENT_TYPE, "222"));

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
    async fn client_basic_auth() {
        let req = Client::new().get("/").basic_auth("username", "password");
        assert_eq!(
            req.head
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Basic dXNlcm5hbWU6cGFzc3dvcmQ="
        );

        let req = Client::new().get("/").basic_auth("username", "");
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
    async fn client_bearer_auth() {
        let req = Client::new().get("/").bearer_auth("someS3cr3tAutht0k3n");
        assert_eq!(
            req.head
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer someS3cr3tAutht0k3n"
        );
    }

    #[actix_rt::test]
    async fn client_query() {
        let req = Client::new()
            .get("/")
            .query(&[("key1", "val1"), ("key2", "val2")])
            .unwrap();
        assert_eq!(req.get_uri().query().unwrap(), "key1=val1&key2=val2");
    }
}
