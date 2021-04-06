use std::{
    cell::{Ref, RefMut},
    convert::TryInto,
};

use actix_http::{
    body::{Body, BodyStream},
    http::{
        header::{self, HeaderName, IntoHeaderPair, IntoHeaderValue},
        ConnectionType, Error as HttpError, StatusCode,
    },
    Extensions, Response, ResponseHead,
};
use bytes::Bytes;
use futures_core::Stream;
use serde::Serialize;

use crate::error::Error;

// pub struct HttpResponse<B = dev::Body>(dev::BaseHttpResponse<B>);

// impl HttpResponse {
//     /// Create HTTP response builder with specific status.
//     #[inline]
//     pub fn build(status: http::StatusCode) -> HttpResponseBuilder {
//         HttpResponseBuilder(dev::BaseHttpResponse::build(status))
//     }

//     /// Constructs a response
//     #[inline]
//     pub fn new(status: http::StatusCode) -> HttpResponse {
//         HttpResponse(dev::BaseHttpResponse::new(status))
//     }

//     /// Constructs an error response
//     #[inline]
//     pub fn from_error(error: Error) -> HttpResponse {
//         HttpResponse(dev::BaseHttpResponse::from_error(error))
//     }
// }

// impl ops::Deref for HttpResponse {
//     type Target = dev::BaseHttpResponse;

//     fn deref(&self) -> &Self::Target {
//         &self.0
//     }
// }

// impl ops::DerefMut for HttpResponse {
//     fn deref_mut(&mut self) -> &mut Self::Target {
//         &mut self.0
//     }
// }

// impl<B> From<HttpResponse<B>> for dev::BaseHttpResponse<B> {
//     fn from(res: HttpResponse<B>) -> Self {
//         res.0
//     }
// }

/// An HTTP response builder.
///
/// This type can be used to construct an instance of `Response` through a builder-like pattern.
pub struct HttpResponseBuilder {
    head: Option<ResponseHead>,
    err: Option<HttpError>,
}

impl HttpResponseBuilder {
    #[inline]
    /// Create response builder
    pub fn new(status: StatusCode) -> Self {
        Self {
            head: Some(ResponseHead::new(status)),
            err: None,
        }
    }

    /// Set HTTP status code of this response.
    #[inline]
    pub fn status(&mut self, status: StatusCode) -> &mut Self {
        if let Some(parts) = self.parts() {
            parts.status = status;
        }
        self
    }

    /// Insert a header, replacing any that were set with an equivalent field name.
    ///
    /// ```
    /// # use actix_http::Response;
    /// use actix_http::http::header;
    ///
    /// Response::Ok()
    ///     .insert_header((header::CONTENT_TYPE, mime::APPLICATION_JSON))
    ///     .insert_header(("X-TEST", "value"))
    ///     .finish();
    /// ```
    pub fn insert_header<H>(&mut self, header: H) -> &mut Self
    where
        H: IntoHeaderPair,
    {
        if let Some(parts) = self.parts() {
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
    /// # use actix_http::Response;
    /// use actix_http::http::header;
    ///
    /// Response::Ok()
    ///     .append_header((header::CONTENT_TYPE, mime::APPLICATION_JSON))
    ///     .append_header(("X-TEST", "value1"))
    ///     .append_header(("X-TEST", "value2"))
    ///     .finish();
    /// ```
    pub fn append_header<H>(&mut self, header: H) -> &mut Self
    where
        H: IntoHeaderPair,
    {
        if let Some(parts) = self.parts() {
            match header.try_into_header_pair() {
                Ok((key, value)) => parts.headers.append(key, value),
                Err(e) => self.err = Some(e.into()),
            };
        }

        self
    }

    /// Replaced with [`Self::insert_header()`].
    #[deprecated = "Replaced with `insert_header((key, value))`."]
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
    #[deprecated = "Replaced with `append_header((key, value))`."]
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
        if let Some(parts) = self.parts() {
            parts.reason = Some(reason);
        }
        self
    }

    /// Set connection type to KeepAlive
    #[inline]
    pub fn keep_alive(&mut self) -> &mut Self {
        if let Some(parts) = self.parts() {
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
        if let Some(parts) = self.parts() {
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
        if let Some(parts) = self.parts() {
            parts.set_connection_type(ConnectionType::Close);
        }
        self
    }

    /// Disable chunked transfer encoding for HTTP/1.1 streaming responses.
    #[inline]
    pub fn no_chunking(&mut self, len: u64) -> &mut Self {
        let mut buf = itoa::Buffer::new();
        self.insert_header((header::CONTENT_LENGTH, buf.format(len)));

        if let Some(parts) = self.parts() {
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
        if let Some(parts) = self.parts() {
            match value.try_into_value() {
                Ok(value) => {
                    parts.headers.insert(header::CONTENT_TYPE, value);
                }
                Err(e) => self.err = Some(e.into()),
            };
        }
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
    /// `ResponseBuilder` can not be used after this call.
    #[inline]
    pub fn body<B: Into<Body>>(&mut self, body: B) -> Response {
        self.message_body(body.into())
    }

    /// Set a body and generate `Response`.
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn message_body<B>(&mut self, body: B) -> Response<B> {
        if let Some(e) = self.err.take() {
            return Response::from(Error::from(e)).into_body();
        }

        // allow unused mut when cookies feature is disabled
        #[allow(unused_mut)]
        let mut head = self.head.take().expect("cannot reuse response builder");

        let mut res = Response::with_body(StatusCode::OK, body);
        *res.head_mut() = head;
        res
    }

    /// Set a streaming body and generate `Response`.
    ///
    /// `ResponseBuilder` can not be used after this call.
    #[inline]
    pub fn streaming<S, E>(&mut self, stream: S) -> Response
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + 'static,
        E: Into<Error> + 'static,
    {
        self.body(Body::from_message(BodyStream::new(stream)))
    }

    /// Set a json body and generate `Response`
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn json(&mut self, value: impl Serialize) -> Response {
        match serde_json::to_string(&value) {
            Ok(body) => {
                let contains = if let Some(parts) = self.parts() {
                    parts.headers.contains_key(header::CONTENT_TYPE)
                } else {
                    true
                };

                if !contains {
                    self.insert_header((header::CONTENT_TYPE, mime::APPLICATION_JSON));
                }

                self.body(Body::from(body))
            }
            Err(e) => Error::from(e).into(),
        }
    }

    /// Set an empty body and generate `Response`
    ///
    /// `ResponseBuilder` can not be used after this call.
    #[inline]
    pub fn finish(&mut self) -> Response {
        self.body(Body::Empty)
    }

    /// This method construct new `ResponseBuilder`
    pub fn take(&mut self) -> Self {
        Self {
            head: self.head.take(),
            err: self.err.take(),
        }
    }

    #[inline]
    fn parts(&mut self) -> Option<&mut ResponseHead> {
        if self.err.is_some() {
            return None;
        }

        self.head.as_mut()
    }
}

// mod http_codes {
//     //! Status code based HTTP response builders.

//     use actix_http::http::StatusCode;

//     use super::{HttpResponse, HttpResponseBuilder};

//     macro_rules! static_resp {
//         ($name:ident, $status:expr) => {
//             #[allow(non_snake_case, missing_docs)]
//             pub fn $name() -> HttpResponseBuilder {
//                 HttpResponseBuilder::new($status)
//             }
//         };
//     }

//     impl HttpResponse {
//         static_resp!(Continue, StatusCode::CONTINUE);
//         static_resp!(SwitchingProtocols, StatusCode::SWITCHING_PROTOCOLS);
//         static_resp!(Processing, StatusCode::PROCESSING);

//         static_resp!(Ok, StatusCode::OK);
//         static_resp!(Created, StatusCode::CREATED);
//         static_resp!(Accepted, StatusCode::ACCEPTED);
//         static_resp!(
//             NonAuthoritativeInformation,
//             StatusCode::NON_AUTHORITATIVE_INFORMATION
//         );

//         static_resp!(NoContent, StatusCode::NO_CONTENT);
//         static_resp!(ResetContent, StatusCode::RESET_CONTENT);
//         static_resp!(PartialContent, StatusCode::PARTIAL_CONTENT);
//         static_resp!(MultiStatus, StatusCode::MULTI_STATUS);
//         static_resp!(AlreadyReported, StatusCode::ALREADY_REPORTED);

//         static_resp!(MultipleChoices, StatusCode::MULTIPLE_CHOICES);
//         static_resp!(MovedPermanently, StatusCode::MOVED_PERMANENTLY);
//         static_resp!(Found, StatusCode::FOUND);
//         static_resp!(SeeOther, StatusCode::SEE_OTHER);
//         static_resp!(NotModified, StatusCode::NOT_MODIFIED);
//         static_resp!(UseProxy, StatusCode::USE_PROXY);
//         static_resp!(TemporaryRedirect, StatusCode::TEMPORARY_REDIRECT);
//         static_resp!(PermanentRedirect, StatusCode::PERMANENT_REDIRECT);

//         static_resp!(BadRequest, StatusCode::BAD_REQUEST);
//         static_resp!(NotFound, StatusCode::NOT_FOUND);
//         static_resp!(Unauthorized, StatusCode::UNAUTHORIZED);
//         static_resp!(PaymentRequired, StatusCode::PAYMENT_REQUIRED);
//         static_resp!(Forbidden, StatusCode::FORBIDDEN);
//         static_resp!(MethodNotAllowed, StatusCode::METHOD_NOT_ALLOWED);
//         static_resp!(NotAcceptable, StatusCode::NOT_ACCEPTABLE);
//         static_resp!(
//             ProxyAuthenticationRequired,
//             StatusCode::PROXY_AUTHENTICATION_REQUIRED
//         );
//         static_resp!(RequestTimeout, StatusCode::REQUEST_TIMEOUT);
//         static_resp!(Conflict, StatusCode::CONFLICT);
//         static_resp!(Gone, StatusCode::GONE);
//         static_resp!(LengthRequired, StatusCode::LENGTH_REQUIRED);
//         static_resp!(PreconditionFailed, StatusCode::PRECONDITION_FAILED);
//         static_resp!(PreconditionRequired, StatusCode::PRECONDITION_REQUIRED);
//         static_resp!(PayloadTooLarge, StatusCode::PAYLOAD_TOO_LARGE);
//         static_resp!(UriTooLong, StatusCode::URI_TOO_LONG);
//         static_resp!(UnsupportedMediaType, StatusCode::UNSUPPORTED_MEDIA_TYPE);
//         static_resp!(RangeNotSatisfiable, StatusCode::RANGE_NOT_SATISFIABLE);
//         static_resp!(ExpectationFailed, StatusCode::EXPECTATION_FAILED);
//         static_resp!(UnprocessableEntity, StatusCode::UNPROCESSABLE_ENTITY);
//         static_resp!(TooManyRequests, StatusCode::TOO_MANY_REQUESTS);
//         static_resp!(
//             RequestHeaderFieldsTooLarge,
//             StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
//         );
//         static_resp!(
//             UnavailableForLegalReasons,
//             StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS
//         );

//         static_resp!(InternalServerError, StatusCode::INTERNAL_SERVER_ERROR);
//         static_resp!(NotImplemented, StatusCode::NOT_IMPLEMENTED);
//         static_resp!(BadGateway, StatusCode::BAD_GATEWAY);
//         static_resp!(ServiceUnavailable, StatusCode::SERVICE_UNAVAILABLE);
//         static_resp!(GatewayTimeout, StatusCode::GATEWAY_TIMEOUT);
//         static_resp!(VersionNotSupported, StatusCode::HTTP_VERSION_NOT_SUPPORTED);
//         static_resp!(VariantAlsoNegotiates, StatusCode::VARIANT_ALSO_NEGOTIATES);
//         static_resp!(InsufficientStorage, StatusCode::INSUFFICIENT_STORAGE);
//         static_resp!(LoopDetected, StatusCode::LOOP_DETECTED);
//     }

//     #[cfg(test)]
//     mod tests {
//         use crate::dev::Body;
//         use crate::http::StatusCode;
//         use crate::HttpRespone;

//         #[test]
//         fn test_build() {
//             let resp = HttpResponse::Ok().body(Body::Empty);
//             assert_eq!(resp.status(), StatusCode::OK);
//         }
//     }
// }
