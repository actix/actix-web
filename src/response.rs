use std::{
    cell::{Ref, RefMut},
    convert::TryInto,
    fmt,
    future::Future,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::{
    body::{Body, BodyStream, MessageBody, ResponseBody},
    http::{
        header::{self, HeaderMap, HeaderName, IntoHeaderPair, IntoHeaderValue},
        ConnectionType, Error as HttpError, StatusCode,
    },
    Extensions, Response, ResponseHead,
};
use bytes::Bytes;
use futures_core::Stream;
use serde::Serialize;

#[cfg(feature = "cookies")]
use actix_http::http::header::HeaderValue;
#[cfg(feature = "cookies")]
use cookie::{Cookie, CookieJar};

use crate::error::{Error, JsonPayloadError};

/// An HTTP Response
pub struct HttpResponse<B = Body> {
    res: Response<B>,
    error: Option<Error>,
}

impl HttpResponse<Body> {
    /// Create HTTP response builder with specific status.
    #[inline]
    pub fn build(status: StatusCode) -> HttpResponseBuilder {
        HttpResponseBuilder::new(status)
    }

    /// Create HTTP response builder
    #[inline]
    pub fn build_from<T: Into<HttpResponseBuilder>>(source: T) -> HttpResponseBuilder {
        source.into()
    }

    /// Create a response.
    #[inline]
    pub fn new(status: StatusCode) -> Self {
        Self {
            res: Response::new(status),
            error: None,
        }
    }

    /// Create an error response.
    #[inline]
    pub fn from_error(error: Error) -> Self {
        let res = error.as_response_error().error_response();

        Self {
            res,
            error: Some(error),
        }
    }

    /// Convert response to response with body
    pub fn into_body<B>(self) -> HttpResponse<B> {
        HttpResponse {
            res: self.res.into_body(),
            error: self.error,
        }
    }
}

impl<B> HttpResponse<B> {
    /// Constructs a response with body
    #[inline]
    pub fn with_body(status: StatusCode, body: B) -> Self {
        Self {
            res: Response::with_body(status, body),
            error: None,
        }
    }

    /// Returns a reference to response head.
    #[inline]
    pub fn head(&self) -> &ResponseHead {
        self.res.head()
    }

    /// Returns a mutable reference to response head.
    #[inline]
    pub fn head_mut(&mut self) -> &mut ResponseHead {
        self.res.head_mut()
    }

    /// The source `error` for this response
    #[inline]
    pub fn error(&self) -> Option<&Error> {
        self.error.as_ref()
    }

    /// Get the response status code
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.res.status()
    }

    /// Set the `StatusCode` for this response
    #[inline]
    pub fn status_mut(&mut self) -> &mut StatusCode {
        self.res.status_mut()
    }

    /// Get the headers from the response
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        self.res.headers()
    }

    /// Get a mutable reference to the headers
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        self.res.headers_mut()
    }

    /// Get an iterator for the cookies set by this response.
    #[cfg(feature = "cookies")]
    pub fn cookies(&self) -> CookieIter<'_> {
        CookieIter {
            iter: self.headers().get_all(header::SET_COOKIE),
        }
    }

    /// Add a cookie to this response
    #[cfg(feature = "cookies")]
    pub fn add_cookie(&mut self, cookie: &Cookie<'_>) -> Result<(), HttpError> {
        HeaderValue::from_str(&cookie.to_string())
            .map(|c| {
                self.headers_mut().append(header::SET_COOKIE, c);
            })
            .map_err(|e| e.into())
    }

    /// Remove all cookies with the given name from this response. Returns
    /// the number of cookies removed.
    #[cfg(feature = "cookies")]
    pub fn del_cookie(&mut self, name: &str) -> usize {
        let headers = self.headers_mut();

        let vals: Vec<HeaderValue> = headers
            .get_all(header::SET_COOKIE)
            .map(|v| v.to_owned())
            .collect();

        headers.remove(header::SET_COOKIE);

        let mut count: usize = 0;
        for v in vals {
            if let Ok(s) = v.to_str() {
                if let Ok(c) = Cookie::parse_encoded(s) {
                    if c.name() == name {
                        count += 1;
                        continue;
                    }
                }
            }

            // put set-cookie header head back if it does not validate
            headers.append(header::SET_COOKIE, v);
        }

        count
    }

    /// Connection upgrade status
    #[inline]
    pub fn upgrade(&self) -> bool {
        self.res.upgrade()
    }

    /// Keep-alive status for this connection
    pub fn keep_alive(&self) -> bool {
        self.res.keep_alive()
    }

    /// Responses extensions
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        self.res.extensions()
    }

    /// Mutable reference to a the response's extensions
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<'_, Extensions> {
        self.res.extensions_mut()
    }

    /// Get body of this response
    #[inline]
    pub fn body(&self) -> &ResponseBody<B> {
        self.res.body()
    }

    /// Set a body
    pub fn set_body<B2>(self, body: B2) -> HttpResponse<B2> {
        HttpResponse {
            res: self.res.set_body(body),
            error: None,
            // error: self.error, ??
        }
    }

    /// Split response and body
    pub fn into_parts(self) -> (HttpResponse<()>, ResponseBody<B>) {
        let (head, body) = self.res.into_parts();

        (
            HttpResponse {
                res: head,
                error: None,
            },
            body,
        )
    }

    /// Drop request's body
    pub fn drop_body(self) -> HttpResponse<()> {
        HttpResponse {
            res: self.res.drop_body(),
            error: None,
        }
    }

    /// Set a body and return previous body value
    pub fn map_body<F, B2>(self, f: F) -> HttpResponse<B2>
    where
        F: FnOnce(&mut ResponseHead, ResponseBody<B>) -> ResponseBody<B2>,
    {
        HttpResponse {
            res: self.res.map_body(f),
            error: self.error,
        }
    }

    /// Extract response body
    pub fn take_body(&mut self) -> ResponseBody<B> {
        self.res.take_body()
    }
}

impl<B: MessageBody> fmt::Debug for HttpResponse<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("error", &self.error)
            .field("res", &self.res)
            .finish()
    }
}

impl<B> From<Response<B>> for HttpResponse<B> {
    fn from(res: Response<B>) -> Self {
        HttpResponse { res, error: None }
    }
}

impl From<Error> for HttpResponse {
    fn from(err: Error) -> Self {
        HttpResponse::from_error(err)
    }
}

impl<B> From<HttpResponse<B>> for Response<B> {
    fn from(res: HttpResponse<B>) -> Self {
        // this impl will always be called as part of dispatcher

        // TODO: expose cause somewhere?
        // if let Some(err) = res.error {
        //     eprintln!("impl<B> From<HttpResponse<B>> for Response<B> let Some(err)");
        //     return Response::from_error(err).into_body();
        // }

        res.res
    }
}

impl Future for HttpResponse {
    type Output = Result<Response<Body>, Error>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(err) = self.error.take() {
            return Poll::Ready(Ok(Response::from_error(err).into_body()));
        }

        Poll::Ready(Ok(mem::replace(
            &mut self.res,
            Response::new(StatusCode::default()),
        )))
    }
}

/// An HTTP response builder.
///
/// This type can be used to construct an instance of `Response` through a builder-like pattern.
pub struct HttpResponseBuilder {
    head: Option<ResponseHead>,
    err: Option<HttpError>,
    #[cfg(feature = "cookies")]
    cookies: Option<CookieJar>,
}

impl HttpResponseBuilder {
    #[inline]
    /// Create response builder
    pub fn new(status: StatusCode) -> Self {
        Self {
            head: Some(ResponseHead::new(status)),
            err: None,
            #[cfg(feature = "cookies")]
            cookies: None,
        }
    }

    /// Set HTTP status code of this response.
    #[inline]
    pub fn status(&mut self, status: StatusCode) -> &mut Self {
        if let Some(parts) = self.inner() {
            parts.status = status;
        }
        self
    }

    /// Insert a header, replacing any that were set with an equivalent field name.
    ///
    /// ```
    /// use actix_web::{HttpResponse, http::header};
    ///
    /// HttpResponse::Ok()
    ///     .insert_header(header::ContentType(mime::APPLICATION_JSON))
    ///     .insert_header(("X-TEST", "value"))
    ///     .finish();
    /// ```
    pub fn insert_header<H>(&mut self, header: H) -> &mut Self
    where
        H: IntoHeaderPair,
    {
        if let Some(parts) = self.inner() {
            match header.try_into_header_pair() {
                Ok((key, value)) => {
                    parts.headers.insert(key, value);
                }
                Err(e) => self.err = Some(e.into()),
            };
        }

        self
    }

    /// Append a header, keeping any that were set with an equivalent field name.
    ///
    /// ```
    /// use actix_web::{HttpResponse, http::header};
    ///
    /// HttpResponse::Ok()
    ///     .append_header(header::ContentType(mime::APPLICATION_JSON))
    ///     .append_header(("X-TEST", "value1"))
    ///     .append_header(("X-TEST", "value2"))
    ///     .finish();
    /// ```
    pub fn append_header<H>(&mut self, header: H) -> &mut Self
    where
        H: IntoHeaderPair,
    {
        if let Some(parts) = self.inner() {
            match header.try_into_header_pair() {
                Ok((key, value)) => parts.headers.append(key, value),
                Err(e) => self.err = Some(e.into()),
            };
        }

        self
    }

    /// Replaced with [`Self::insert_header()`].
    #[deprecated(
        since = "4.0.0",
        note = "Replaced with `insert_header((key, value))`. Will be removed in v5."
    )]
    pub fn set_header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: TryInto<HeaderName>,
        K::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        if self.err.is_some() {
            return self;
        }

        match (key.try_into(), value.try_into_value()) {
            (Ok(name), Ok(value)) => return self.insert_header((name, value)),
            (Err(err), _) => self.err = Some(err.into()),
            (_, Err(err)) => self.err = Some(err.into()),
        }

        self
    }

    /// Replaced with [`Self::append_header()`].
    #[deprecated(
        since = "4.0.0",
        note = "Replaced with `append_header((key, value))`. Will be removed in v5."
    )]
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: TryInto<HeaderName>,
        K::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        if self.err.is_some() {
            return self;
        }

        match (key.try_into(), value.try_into_value()) {
            (Ok(name), Ok(value)) => return self.append_header((name, value)),
            (Err(err), _) => self.err = Some(err.into()),
            (_, Err(err)) => self.err = Some(err.into()),
        }

        self
    }

    /// Set the custom reason for the response.
    #[inline]
    pub fn reason(&mut self, reason: &'static str) -> &mut Self {
        if let Some(parts) = self.inner() {
            parts.reason = Some(reason);
        }
        self
    }

    /// Set connection type to KeepAlive
    #[inline]
    pub fn keep_alive(&mut self) -> &mut Self {
        if let Some(parts) = self.inner() {
            parts.set_connection_type(ConnectionType::KeepAlive);
        }
        self
    }

    /// Set connection type to Upgrade
    #[inline]
    pub fn upgrade<V>(&mut self, value: V) -> &mut Self
    where
        V: IntoHeaderValue,
    {
        if let Some(parts) = self.inner() {
            parts.set_connection_type(ConnectionType::Upgrade);
        }

        if let Ok(value) = value.try_into_value() {
            self.insert_header((header::UPGRADE, value));
        }

        self
    }

    /// Force close connection, even if it is marked as keep-alive
    #[inline]
    pub fn force_close(&mut self) -> &mut Self {
        if let Some(parts) = self.inner() {
            parts.set_connection_type(ConnectionType::Close);
        }
        self
    }

    /// Disable chunked transfer encoding for HTTP/1.1 streaming responses.
    #[inline]
    pub fn no_chunking(&mut self, len: u64) -> &mut Self {
        let mut buf = itoa::Buffer::new();
        self.insert_header((header::CONTENT_LENGTH, buf.format(len)));

        if let Some(parts) = self.inner() {
            parts.no_chunking(true);
        }
        self
    }

    /// Set response content type.
    #[inline]
    pub fn content_type<V>(&mut self, value: V) -> &mut Self
    where
        V: IntoHeaderValue,
    {
        if let Some(parts) = self.inner() {
            match value.try_into_value() {
                Ok(value) => {
                    parts.headers.insert(header::CONTENT_TYPE, value);
                }
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set a cookie.
    ///
    /// ```
    /// use actix_web::{HttpResponse, cookie::Cookie};
    ///
    /// HttpResponse::Ok()
    ///     .cookie(
    ///         Cookie::build("name", "value")
    ///             .domain("www.rust-lang.org")
    ///             .path("/")
    ///             .secure(true)
    ///             .http_only(true)
    ///             .finish(),
    ///     )
    ///     .finish();
    /// ```
    #[cfg(feature = "cookies")]
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

    /// Remove cookie.
    ///
    /// A `Set-Cookie` header is added that will delete a cookie with the same name from the client.
    ///
    /// ```
    /// use actix_web::{HttpRequest, HttpResponse, Responder};
    ///
    /// async fn handler(req: HttpRequest) -> impl Responder {
    ///     let mut builder = HttpResponse::Ok();
    ///
    ///     if let Some(ref cookie) = req.cookie("name") {
    ///         builder.del_cookie(cookie);
    ///     }
    ///
    ///     builder.finish()
    /// }
    /// ```
    #[cfg(feature = "cookies")]
    pub fn del_cookie(&mut self, cookie: &Cookie<'_>) -> &mut Self {
        if self.cookies.is_none() {
            self.cookies = Some(CookieJar::new())
        }
        let jar = self.cookies.as_mut().unwrap();
        let cookie = cookie.clone().into_owned();
        jar.add_original(cookie.clone());
        jar.remove(cookie);
        self
    }

    /// Responses extensions
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        let head = self.head.as_ref().expect("cannot reuse response builder");
        head.extensions()
    }

    /// Mutable reference to a the response's extensions
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<'_, Extensions> {
        let head = self.head.as_ref().expect("cannot reuse response builder");
        head.extensions_mut()
    }

    /// Set a body and generate `Response`.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    #[inline]
    pub fn body<B: Into<Body>>(&mut self, body: B) -> HttpResponse {
        self.message_body(body.into())
    }

    /// Set a body and generate `Response`.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    pub fn message_body<B>(&mut self, body: B) -> HttpResponse<B> {
        if let Some(err) = self.err.take() {
            return HttpResponse::from_error(Error::from(err)).into_body();
        }

        // allow unused mut when cookies feature is disabled
        #[allow(unused_mut)]
        let mut head = self.head.take().expect("cannot reuse response builder");

        let mut res = HttpResponse::with_body(StatusCode::OK, body);
        *res.head_mut() = head;

        #[cfg(feature = "cookies")]
        if let Some(ref jar) = self.cookies {
            for cookie in jar.delta() {
                match HeaderValue::from_str(&cookie.to_string()) {
                    Ok(val) => res.headers_mut().append(header::SET_COOKIE, val),
                    Err(err) => return HttpResponse::from_error(Error::from(err)).into_body(),
                };
            }
        }

        res
    }

    /// Set a streaming body and generate `Response`.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    #[inline]
    pub fn streaming<S, E>(&mut self, stream: S) -> HttpResponse
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + 'static,
        E: Into<Error> + 'static,
    {
        self.body(Body::from_message(BodyStream::new(stream)))
    }

    /// Set a json body and generate `Response`
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    pub fn json(&mut self, value: impl Serialize) -> HttpResponse {
        match serde_json::to_string(&value) {
            Ok(body) => {
                let contains = if let Some(parts) = self.inner() {
                    parts.headers.contains_key(header::CONTENT_TYPE)
                } else {
                    true
                };

                if !contains {
                    self.insert_header((header::CONTENT_TYPE, mime::APPLICATION_JSON));
                }

                self.body(Body::from(body))
            }
            Err(err) => HttpResponse::from_error(JsonPayloadError::Serialize(err).into()),
        }
    }

    /// Set an empty body and generate `Response`
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    #[inline]
    pub fn finish(&mut self) -> HttpResponse {
        self.body(Body::Empty)
    }

    /// This method construct new `HttpResponseBuilder`
    pub fn take(&mut self) -> Self {
        Self {
            head: self.head.take(),
            err: self.err.take(),
            #[cfg(feature = "cookies")]
            cookies: self.cookies.take(),
        }
    }

    #[inline]
    fn inner(&mut self) -> Option<&mut ResponseHead> {
        if self.err.is_some() {
            return None;
        }

        self.head.as_mut()
    }
}

impl From<HttpResponseBuilder> for HttpResponse {
    fn from(mut builder: HttpResponseBuilder) -> Self {
        builder.finish()
    }
}

impl From<HttpResponseBuilder> for Response<Body> {
    fn from(mut builder: HttpResponseBuilder) -> Self {
        builder.finish().into()
    }
}

impl Future for HttpResponseBuilder {
    type Output = Result<HttpResponse, Error>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        eprintln!("httpresponse future error");
        Poll::Ready(Ok(self.finish()))
    }
}

#[cfg(feature = "cookies")]
pub struct CookieIter<'a> {
    iter: header::GetAll<'a>,
}

#[cfg(feature = "cookies")]
impl<'a> Iterator for CookieIter<'a> {
    type Item = Cookie<'a>;

    #[inline]
    fn next(&mut self) -> Option<Cookie<'a>> {
        for v in self.iter.by_ref() {
            if let Ok(c) = Cookie::parse_encoded(v.to_str().ok()?) {
                return Some(c);
            }
        }
        None
    }
}

mod http_codes {
    //! Status code based HTTP response builders.

    use actix_http::http::StatusCode;

    use super::{HttpResponse, HttpResponseBuilder};

    macro_rules! static_resp {
        ($name:ident, $status:expr) => {
            #[allow(non_snake_case, missing_docs)]
            pub fn $name() -> HttpResponseBuilder {
                HttpResponseBuilder::new($status)
            }
        };
    }

    impl HttpResponse {
        static_resp!(Continue, StatusCode::CONTINUE);
        static_resp!(SwitchingProtocols, StatusCode::SWITCHING_PROTOCOLS);
        static_resp!(Processing, StatusCode::PROCESSING);

        static_resp!(Ok, StatusCode::OK);
        static_resp!(Created, StatusCode::CREATED);
        static_resp!(Accepted, StatusCode::ACCEPTED);
        static_resp!(
            NonAuthoritativeInformation,
            StatusCode::NON_AUTHORITATIVE_INFORMATION
        );

        static_resp!(NoContent, StatusCode::NO_CONTENT);
        static_resp!(ResetContent, StatusCode::RESET_CONTENT);
        static_resp!(PartialContent, StatusCode::PARTIAL_CONTENT);
        static_resp!(MultiStatus, StatusCode::MULTI_STATUS);
        static_resp!(AlreadyReported, StatusCode::ALREADY_REPORTED);

        static_resp!(MultipleChoices, StatusCode::MULTIPLE_CHOICES);
        static_resp!(MovedPermanently, StatusCode::MOVED_PERMANENTLY);
        static_resp!(Found, StatusCode::FOUND);
        static_resp!(SeeOther, StatusCode::SEE_OTHER);
        static_resp!(NotModified, StatusCode::NOT_MODIFIED);
        static_resp!(UseProxy, StatusCode::USE_PROXY);
        static_resp!(TemporaryRedirect, StatusCode::TEMPORARY_REDIRECT);
        static_resp!(PermanentRedirect, StatusCode::PERMANENT_REDIRECT);

        static_resp!(BadRequest, StatusCode::BAD_REQUEST);
        static_resp!(NotFound, StatusCode::NOT_FOUND);
        static_resp!(Unauthorized, StatusCode::UNAUTHORIZED);
        static_resp!(PaymentRequired, StatusCode::PAYMENT_REQUIRED);
        static_resp!(Forbidden, StatusCode::FORBIDDEN);
        static_resp!(MethodNotAllowed, StatusCode::METHOD_NOT_ALLOWED);
        static_resp!(NotAcceptable, StatusCode::NOT_ACCEPTABLE);
        static_resp!(
            ProxyAuthenticationRequired,
            StatusCode::PROXY_AUTHENTICATION_REQUIRED
        );
        static_resp!(RequestTimeout, StatusCode::REQUEST_TIMEOUT);
        static_resp!(Conflict, StatusCode::CONFLICT);
        static_resp!(Gone, StatusCode::GONE);
        static_resp!(LengthRequired, StatusCode::LENGTH_REQUIRED);
        static_resp!(PreconditionFailed, StatusCode::PRECONDITION_FAILED);
        static_resp!(PreconditionRequired, StatusCode::PRECONDITION_REQUIRED);
        static_resp!(PayloadTooLarge, StatusCode::PAYLOAD_TOO_LARGE);
        static_resp!(UriTooLong, StatusCode::URI_TOO_LONG);
        static_resp!(UnsupportedMediaType, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        static_resp!(RangeNotSatisfiable, StatusCode::RANGE_NOT_SATISFIABLE);
        static_resp!(ExpectationFailed, StatusCode::EXPECTATION_FAILED);
        static_resp!(UnprocessableEntity, StatusCode::UNPROCESSABLE_ENTITY);
        static_resp!(TooManyRequests, StatusCode::TOO_MANY_REQUESTS);
        static_resp!(
            RequestHeaderFieldsTooLarge,
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
        );
        static_resp!(
            UnavailableForLegalReasons,
            StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS
        );

        static_resp!(InternalServerError, StatusCode::INTERNAL_SERVER_ERROR);
        static_resp!(NotImplemented, StatusCode::NOT_IMPLEMENTED);
        static_resp!(BadGateway, StatusCode::BAD_GATEWAY);
        static_resp!(ServiceUnavailable, StatusCode::SERVICE_UNAVAILABLE);
        static_resp!(GatewayTimeout, StatusCode::GATEWAY_TIMEOUT);
        static_resp!(VersionNotSupported, StatusCode::HTTP_VERSION_NOT_SUPPORTED);
        static_resp!(VariantAlsoNegotiates, StatusCode::VARIANT_ALSO_NEGOTIATES);
        static_resp!(InsufficientStorage, StatusCode::INSUFFICIENT_STORAGE);
        static_resp!(LoopDetected, StatusCode::LOOP_DETECTED);
    }

    #[cfg(test)]
    mod tests {
        use crate::dev::Body;
        use crate::http::StatusCode;
        use crate::HttpResponse;

        #[test]
        fn test_build() {
            let resp = HttpResponse::Ok().body(Body::Empty);
            assert_eq!(resp.status(), StatusCode::OK);
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::{Bytes, BytesMut};

    use super::{HttpResponse, HttpResponseBuilder};
    use crate::dev::{Body, MessageBody, ResponseBody};
    use crate::http::header::{self, HeaderValue, CONTENT_TYPE, COOKIE};
    use crate::http::StatusCode;

    #[test]
    fn test_debug() {
        let resp = HttpResponse::Ok()
            .append_header((COOKIE, HeaderValue::from_static("cookie1=value1; ")))
            .append_header((COOKIE, HeaderValue::from_static("cookie2=value2; ")))
            .finish();
        let dbg = format!("{:?}", resp);
        assert!(dbg.contains("HttpResponse"));
    }

    #[test]
    fn test_basic_builder() {
        let resp = HttpResponse::Ok()
            .insert_header(("X-TEST", "value"))
            .finish();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_upgrade() {
        let resp = HttpResponseBuilder::new(StatusCode::OK)
            .upgrade("websocket")
            .finish();
        assert!(resp.upgrade());
        assert_eq!(
            resp.headers().get(header::UPGRADE).unwrap(),
            HeaderValue::from_static("websocket")
        );
    }

    #[test]
    fn test_force_close() {
        let resp = HttpResponseBuilder::new(StatusCode::OK)
            .force_close()
            .finish();
        assert!(!resp.keep_alive())
    }

    #[test]
    fn test_content_type() {
        let resp = HttpResponseBuilder::new(StatusCode::OK)
            .content_type("text/plain")
            .body(Body::Empty);
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "text/plain")
    }

    pub async fn read_body<B>(mut body: ResponseBody<B>) -> Bytes
    where
        B: MessageBody + Unpin,
    {
        use futures_util::StreamExt as _;

        let mut bytes = BytesMut::new();
        while let Some(item) = body.next().await {
            bytes.extend_from_slice(&item.unwrap());
        }
        bytes.freeze()
    }

    #[actix_rt::test]
    async fn test_json() {
        let mut resp = HttpResponse::Ok().json(vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("application/json"));
        assert_eq!(
            read_body(resp.take_body()).await.as_ref(),
            br#"["v1","v2","v3"]"#
        );

        let mut resp = HttpResponse::Ok().json(&["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("application/json"));
        assert_eq!(
            read_body(resp.take_body()).await.as_ref(),
            br#"["v1","v2","v3"]"#
        );

        // content type override
        let mut resp = HttpResponse::Ok()
            .insert_header((CONTENT_TYPE, "text/json"))
            .json(&vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("text/json"));
        assert_eq!(
            read_body(resp.take_body()).await.as_ref(),
            br#"["v1","v2","v3"]"#
        );
    }

    #[actix_rt::test]
    async fn test_serde_json_in_body() {
        let mut resp = HttpResponse::Ok().body(
            serde_json::to_vec(&serde_json::json!({ "test-key": "test-value" })).unwrap(),
        );

        assert_eq!(
            read_body(resp.take_body()).await.as_ref(),
            br#"{"test-key":"test-value"}"#
        );
    }

    #[test]
    fn response_builder_header_insert_kv() {
        let mut res = HttpResponse::Ok();
        res.insert_header(("Content-Type", "application/octet-stream"));
        let res = res.finish();

        assert_eq!(
            res.headers().get("Content-Type"),
            Some(&HeaderValue::from_static("application/octet-stream"))
        );
    }

    #[test]
    fn response_builder_header_insert_typed() {
        let mut res = HttpResponse::Ok();
        res.insert_header((header::CONTENT_TYPE, mime::APPLICATION_OCTET_STREAM));
        let res = res.finish();

        assert_eq!(
            res.headers().get("Content-Type"),
            Some(&HeaderValue::from_static("application/octet-stream"))
        );
    }

    #[test]
    fn response_builder_header_append_kv() {
        let mut res = HttpResponse::Ok();
        res.append_header(("Content-Type", "application/octet-stream"));
        res.append_header(("Content-Type", "application/json"));
        let res = res.finish();

        let headers: Vec<_> = res.headers().get_all("Content-Type").cloned().collect();
        assert_eq!(headers.len(), 2);
        assert!(headers.contains(&HeaderValue::from_static("application/octet-stream")));
        assert!(headers.contains(&HeaderValue::from_static("application/json")));
    }

    #[test]
    fn response_builder_header_append_typed() {
        let mut res = HttpResponse::Ok();
        res.append_header((header::CONTENT_TYPE, mime::APPLICATION_OCTET_STREAM));
        res.append_header((header::CONTENT_TYPE, mime::APPLICATION_JSON));
        let res = res.finish();

        let headers: Vec<_> = res.headers().get_all("Content-Type").cloned().collect();
        assert_eq!(headers.len(), 2);
        assert!(headers.contains(&HeaderValue::from_static("application/octet-stream")));
        assert!(headers.contains(&HeaderValue::from_static("application/json")));
    }
}
