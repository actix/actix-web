use std::fmt;
use std::fmt::Write as FmtWrite;
use std::io::Write;

use bytes::{BufMut, BytesMut};
use cookie::{Cookie, CookieJar};
use percent_encoding::{percent_encode, USERINFO_ENCODE_SET};
use urlcrate::Url;

use header::{self, Header, IntoHeaderValue};
use http::{
    uri, Error as HttpError, HeaderMap, HeaderName, HeaderValue, HttpTryFrom, Method,
    Uri, Version,
};

/// An HTTP Client Request
///
/// ```rust
/// # extern crate actix_web;
/// # extern crate futures;
/// # extern crate tokio;
/// # use futures::Future;
/// # use std::process;
/// use actix_web::{actix, client};
///
/// fn main() {
///     actix::run(
///         || client::ClientRequest::get("http://www.rust-lang.org") // <- Create request builder
///             .header("User-Agent", "Actix-web")
///             .finish().unwrap()
///             .send()                                    // <- Send http request
///             .map_err(|_| ())
///             .and_then(|response| {                     // <- server http response
///                 println!("Response: {:?}", response);
/// #               actix::System::current().stop();
///                 Ok(())
///             }),
///     );
/// }
/// ```
pub struct ClientRequest {
    uri: Uri,
    method: Method,
    version: Version,
    headers: HeaderMap,
    chunked: bool,
    upgrade: bool,
}

impl Default for ClientRequest {
    fn default() -> ClientRequest {
        ClientRequest {
            uri: Uri::default(),
            method: Method::default(),
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(16),
            chunked: false,
            upgrade: false,
        }
    }
}

impl ClientRequest {
    /// Create request builder for `GET` request
    pub fn get<U: AsRef<str>>(uri: U) -> ClientRequestBuilder {
        let mut builder = ClientRequest::build();
        builder.method(Method::GET).uri(uri);
        builder
    }

    /// Create request builder for `HEAD` request
    pub fn head<U: AsRef<str>>(uri: U) -> ClientRequestBuilder {
        let mut builder = ClientRequest::build();
        builder.method(Method::HEAD).uri(uri);
        builder
    }

    /// Create request builder for `POST` request
    pub fn post<U: AsRef<str>>(uri: U) -> ClientRequestBuilder {
        let mut builder = ClientRequest::build();
        builder.method(Method::POST).uri(uri);
        builder
    }

    /// Create request builder for `PUT` request
    pub fn put<U: AsRef<str>>(uri: U) -> ClientRequestBuilder {
        let mut builder = ClientRequest::build();
        builder.method(Method::PUT).uri(uri);
        builder
    }

    /// Create request builder for `DELETE` request
    pub fn delete<U: AsRef<str>>(uri: U) -> ClientRequestBuilder {
        let mut builder = ClientRequest::build();
        builder.method(Method::DELETE).uri(uri);
        builder
    }
}

impl ClientRequest {
    /// Create client request builder
    pub fn build() -> ClientRequestBuilder {
        ClientRequestBuilder {
            request: Some(ClientRequest::default()),
            err: None,
            cookies: None,
            default_headers: true,
        }
    }

    /// Get the request URI
    #[inline]
    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    /// Set client request URI
    #[inline]
    pub fn set_uri(&mut self, uri: Uri) {
        self.uri = uri
    }

    /// Get the request method
    #[inline]
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// Set HTTP `Method` for the request
    #[inline]
    pub fn set_method(&mut self, method: Method) {
        self.method = method
    }

    /// Get HTTP version for the request
    #[inline]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Set http `Version` for the request
    #[inline]
    pub fn set_version(&mut self, version: Version) {
        self.version = version
    }

    /// Get the headers from the request
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get a mutable reference to the headers
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// is chunked encoding enabled
    #[inline]
    pub fn chunked(&self) -> bool {
        self.chunked
    }

    /// is upgrade request
    #[inline]
    pub fn upgrade(&self) -> bool {
        self.upgrade
    }
}

impl fmt::Debug for ClientRequest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "\nClientRequest {:?} {}:{}",
            self.version, self.method, self.uri
        )?;
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers.iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

/// An HTTP Client request builder
///
/// This type can be used to construct an instance of `ClientRequest` through a
/// builder-like pattern.
pub struct ClientRequestBuilder {
    request: Option<ClientRequest>,
    err: Option<HttpError>,
    cookies: Option<CookieJar>,
    default_headers: bool,
}

impl ClientRequestBuilder {
    /// Set HTTP URI of request.
    #[inline]
    pub fn uri<U: AsRef<str>>(&mut self, uri: U) -> &mut Self {
        match Url::parse(uri.as_ref()) {
            Ok(url) => self._uri(url.as_str()),
            Err(_) => self._uri(uri.as_ref()),
        }
    }

    fn _uri(&mut self, url: &str) -> &mut Self {
        match Uri::try_from(url) {
            Ok(uri) => {
                if let Some(parts) = parts(&mut self.request, &self.err) {
                    parts.uri = uri;
                }
            }
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Set HTTP method of this request.
    #[inline]
    pub fn method(&mut self, method: Method) -> &mut Self {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            parts.method = method;
        }
        self
    }

    /// Set HTTP method of this request.
    #[inline]
    pub fn get_method(&mut self) -> &Method {
        let parts = self.request.as_ref().expect("cannot reuse request builder");
        &parts.method
    }

    /// Set HTTP version of this request.
    ///
    /// By default requests's HTTP version depends on network stream
    #[inline]
    pub fn version(&mut self, version: Version) -> &mut Self {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            parts.version = version;
        }
        self
    }

    /// Set a header.
    ///
    /// ```rust
    /// # extern crate mime;
    /// # extern crate actix_web;
    /// # use actix_web::client::*;
    /// #
    /// use actix_web::{client, http};
    ///
    /// fn main() {
    ///     let req = client::ClientRequest::build()
    ///         .set(http::header::Date::now())
    ///         .set(http::header::ContentType(mime::TEXT_HTML))
    ///         .finish()
    ///         .unwrap();
    /// }
    /// ```
    #[doc(hidden)]
    pub fn set<H: Header>(&mut self, hdr: H) -> &mut Self {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            match hdr.try_into() {
                Ok(value) => {
                    parts.headers.insert(H::name(), value);
                }
                Err(e) => self.err = Some(e.into()),
            }
        }
        self
    }

    /// Append a header.
    ///
    /// Header gets appended to existing header.
    /// To override header use `set_header()` method.
    ///
    /// ```rust
    /// # extern crate http;
    /// # extern crate actix_web;
    /// # use actix_web::client::*;
    /// #
    /// use http::header;
    ///
    /// fn main() {
    ///     let req = ClientRequest::build()
    ///         .header("X-TEST", "value")
    ///         .header(header::CONTENT_TYPE, "application/json")
    ///         .finish()
    ///         .unwrap();
    /// }
    /// ```
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => match value.try_into() {
                    Ok(value) => {
                        parts.headers.append(key, value);
                    }
                    Err(e) => self.err = Some(e.into()),
                },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set a header.
    pub fn set_header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => match value.try_into() {
                    Ok(value) => {
                        parts.headers.insert(key, value);
                    }
                    Err(e) => self.err = Some(e.into()),
                },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set a header only if it is not yet set.
    pub fn set_header_if_none<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => if !parts.headers.contains_key(&key) {
                    match value.try_into() {
                        Ok(value) => {
                            parts.headers.insert(key, value);
                        }
                        Err(e) => self.err = Some(e.into()),
                    }
                },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Enable connection upgrade
    #[inline]
    pub fn upgrade(&mut self) -> &mut Self {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            parts.upgrade = true;
        }
        self
    }

    /// Set request's content type
    #[inline]
    pub fn content_type<V>(&mut self, value: V) -> &mut Self
    where
        HeaderValue: HttpTryFrom<V>,
    {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            match HeaderValue::try_from(value) {
                Ok(value) => {
                    parts.headers.insert(header::CONTENT_TYPE, value);
                }
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set content length
    #[inline]
    pub fn content_length(&mut self, len: u64) -> &mut Self {
        let mut wrt = BytesMut::new().writer();
        let _ = write!(wrt, "{}", len);
        self.header(header::CONTENT_LENGTH, wrt.get_mut().take().freeze())
    }

    /// Set a cookie
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::{client, http};
    ///
    /// fn main() {
    ///     let req = client::ClientRequest::build()
    ///         .cookie(
    ///             http::Cookie::build("name", "value")
    ///                 .domain("www.rust-lang.org")
    ///                 .path("/")
    ///                 .secure(true)
    ///                 .http_only(true)
    ///                 .finish(),
    ///         )
    ///         .finish()
    ///         .unwrap();
    /// }
    /// ```
    pub fn cookie<'c>(&mut self, cookie: Cookie<'c>) -> &mut Self {
        if self.cookies.is_none() {
            let mut jar = CookieJar::new();
            jar.add(cookie.into_owned());
            self.cookies = Some(jar)
        } else {
            self.cookies.as_mut().unwrap().add(cookie.into_owned());
        }
        self
    }

    /// Do not add default request headers.
    /// By default `Accept-Encoding` and `User-Agent` headers are set.
    pub fn no_default_headers(&mut self) -> &mut Self {
        self.default_headers = false;
        self
    }

    /// This method calls provided closure with builder reference if
    /// value is `true`.
    pub fn if_true<F>(&mut self, value: bool, f: F) -> &mut Self
    where
        F: FnOnce(&mut ClientRequestBuilder),
    {
        if value {
            f(self);
        }
        self
    }

    /// This method calls provided closure with builder reference if
    /// value is `Some`.
    pub fn if_some<T, F>(&mut self, value: Option<T>, f: F) -> &mut Self
    where
        F: FnOnce(T, &mut ClientRequestBuilder),
    {
        if let Some(val) = value {
            f(val, self);
        }
        self
    }

    /// Set a body and generate `ClientRequest`.
    ///
    /// `ClientRequestBuilder` can not be used after this call.
    pub fn finish(&mut self) -> Result<ClientRequest, HttpError> {
        if let Some(e) = self.err.take() {
            return Err(e);
        }

        if self.default_headers {
            // enable br only for https
            let https = if let Some(parts) = parts(&mut self.request, &self.err) {
                parts
                    .uri
                    .scheme_part()
                    .map(|s| s == &uri::Scheme::HTTPS)
                    .unwrap_or(true)
            } else {
                true
            };

            if https {
                self.set_header_if_none(header::ACCEPT_ENCODING, "br, gzip, deflate");
            } else {
                self.set_header_if_none(header::ACCEPT_ENCODING, "gzip, deflate");
            }

            // set request host header
            if let Some(parts) = parts(&mut self.request, &self.err) {
                if let Some(host) = parts.uri.host() {
                    if !parts.headers.contains_key(header::HOST) {
                        let mut wrt = BytesMut::with_capacity(host.len() + 5).writer();

                        let _ = match parts.uri.port() {
                            None | Some(80) | Some(443) => write!(wrt, "{}", host),
                            Some(port) => write!(wrt, "{}:{}", host, port),
                        };

                        match wrt.get_mut().take().freeze().try_into() {
                            Ok(value) => {
                                parts.headers.insert(header::HOST, value);
                            }
                            Err(e) => self.err = Some(e.into()),
                        }
                    }
                }
            }

            // user agent
            self.set_header_if_none(
                header::USER_AGENT,
                concat!("actix-http/", env!("CARGO_PKG_VERSION")),
            );
        }

        let mut request = self.request.take().expect("cannot reuse request builder");

        // set cookies
        if let Some(ref mut jar) = self.cookies {
            let mut cookie = String::new();
            for c in jar.delta() {
                let name = percent_encode(c.name().as_bytes(), USERINFO_ENCODE_SET);
                let value = percent_encode(c.value().as_bytes(), USERINFO_ENCODE_SET);
                let _ = write!(&mut cookie, "; {}={}", name, value);
            }
            request.headers.insert(
                header::COOKIE,
                HeaderValue::from_str(&cookie.as_str()[2..]).unwrap(),
            );
        }
        Ok(request)
    }

    /// This method construct new `ClientRequestBuilder`
    pub fn take(&mut self) -> ClientRequestBuilder {
        ClientRequestBuilder {
            request: self.request.take(),
            err: self.err.take(),
            cookies: self.cookies.take(),
            default_headers: self.default_headers,
        }
    }
}

#[inline]
fn parts<'a>(
    parts: &'a mut Option<ClientRequest>,
    err: &Option<HttpError>,
) -> Option<&'a mut ClientRequest> {
    if err.is_some() {
        return None;
    }
    parts.as_mut()
}

impl fmt::Debug for ClientRequestBuilder {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(ref parts) = self.request {
            writeln!(
                f,
                "\nClientRequestBuilder {:?} {}:{}",
                parts.version, parts.method, parts.uri
            )?;
            writeln!(f, "  headers:")?;
            for (key, val) in parts.headers.iter() {
                writeln!(f, "    {:?}: {:?}", key, val)?;
            }
            Ok(())
        } else {
            write!(f, "ClientRequestBuilder(Consumed)")
        }
    }
}
