use actix_http::{
    body::EitherBody,
    error::HttpError,
    header::{HeaderMap, TryIntoHeaderPair},
    StatusCode,
};

use crate::{HttpRequest, HttpResponse, Responder};

/// Allows overriding status code and headers (including cookies) for a [`Responder`].
///
/// Created by calling the [`customize`](Responder::customize) method on a [`Responder`] type.
pub struct CustomizeResponder<R> {
    inner: CustomizeResponderInner<R>,
    error: Option<HttpError>,
}

struct CustomizeResponderInner<R> {
    responder: R,
    status: Option<StatusCode>,
    override_headers: HeaderMap,
    append_headers: HeaderMap,
}

impl<R: Responder> CustomizeResponder<R> {
    pub(crate) fn new(responder: R) -> Self {
        CustomizeResponder {
            inner: CustomizeResponderInner {
                responder,
                status: None,
                override_headers: HeaderMap::new(),
                append_headers: HeaderMap::new(),
            },
            error: None,
        }
    }

    /// Override a status code for the Responder's response.
    ///
    /// # Examples
    /// ```
    /// use actix_web::{Responder, http::StatusCode, test::TestRequest};
    ///
    /// let responder = "Welcome!".customize().with_status(StatusCode::ACCEPTED);
    ///
    /// let request = TestRequest::default().to_http_request();
    /// let response = responder.respond_to(&request);
    /// assert_eq!(response.status(), StatusCode::ACCEPTED);
    /// ```
    pub fn with_status(mut self, status: StatusCode) -> Self {
        if let Some(inner) = self.inner() {
            inner.status = Some(status);
        }

        self
    }

    /// Insert (override) header in the final response.
    ///
    /// Overrides other headers with the same name.
    /// See [`HeaderMap::insert`](crate::http::header::HeaderMap::insert).
    ///
    /// Headers added with this method will be inserted before those added
    /// with [`append_header`](Self::append_header). As such, header(s) can be overridden with more
    /// than one new header by first calling `insert_header` followed by `append_header`.
    ///
    /// # Examples
    /// ```
    /// use actix_web::{Responder, test::TestRequest};
    ///
    /// let responder = "Hello world!"
    ///     .customize()
    ///     .insert_header(("x-version", "1.2.3"));
    ///
    /// let request = TestRequest::default().to_http_request();
    /// let response = responder.respond_to(&request);
    /// assert_eq!(response.headers().get("x-version").unwrap(), "1.2.3");
    /// ```
    pub fn insert_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        if let Some(inner) = self.inner() {
            match header.try_into_pair() {
                Ok((key, value)) => {
                    inner.override_headers.insert(key, value);
                }
                Err(err) => self.error = Some(err.into()),
            };
        }

        self
    }

    /// Append header to the final response.
    ///
    /// Unlike [`insert_header`](Self::insert_header), this will not override existing headers.
    /// See [`HeaderMap::append`](crate::http::header::HeaderMap::append).
    ///
    /// Headers added here are appended _after_ additions/overrides from `insert_header`.
    ///
    /// # Examples
    /// ```
    /// use actix_web::{Responder, test::TestRequest};
    ///
    /// let responder = "Hello world!"
    ///     .customize()
    ///     .append_header(("x-version", "1.2.3"));
    ///
    /// let request = TestRequest::default().to_http_request();
    /// let response = responder.respond_to(&request);
    /// assert_eq!(response.headers().get("x-version").unwrap(), "1.2.3");
    /// ```
    pub fn append_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        if let Some(inner) = self.inner() {
            match header.try_into_pair() {
                Ok((key, value)) => {
                    inner.append_headers.append(key, value);
                }
                Err(err) => self.error = Some(err.into()),
            };
        }

        self
    }

    #[doc(hidden)]
    #[deprecated(since = "4.0.0", note = "Renamed to `insert_header`.")]
    pub fn with_header(self, header: impl TryIntoHeaderPair) -> Self
    where
        Self: Sized,
    {
        self.insert_header(header)
    }

    fn inner(&mut self) -> Option<&mut CustomizeResponderInner<R>> {
        if self.error.is_some() {
            None
        } else {
            Some(&mut self.inner)
        }
    }

    /// Appends a `cookie` to the final response.
    ///
    /// # Errors
    ///
    /// Final response will be an error if `cookie` cannot be converted into a valid header value.
    #[cfg(feature = "cookies")]
    pub fn add_cookie(mut self, cookie: &crate::cookie::Cookie<'_>) -> Self {
        use actix_http::header::{TryIntoHeaderValue as _, SET_COOKIE};

        if let Some(inner) = self.inner() {
            match cookie.to_string().try_into_value() {
                Ok(val) => {
                    inner.append_headers.append(SET_COOKIE, val);
                }
                Err(err) => {
                    self.error = Some(err.into());
                }
            }
        }

        self
    }
}

impl<T> Responder for CustomizeResponder<T>
where
    T: Responder,
{
    type Body = EitherBody<T::Body>;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        if let Some(err) = self.error {
            return HttpResponse::from_error(err).map_into_right_body();
        }

        let mut res = self.inner.responder.respond_to(req);

        if let Some(status) = self.inner.status {
            *res.status_mut() = status;
        }

        for (k, v) in self.inner.override_headers {
            res.headers_mut().insert(k, v);
        }

        for (k, v) in self.inner.append_headers {
            res.headers_mut().append(k, v);
        }

        res.map_into_left_body()
    }
}

#[cfg(test)]
mod tests {
    use actix_http::body::to_bytes;
    use bytes::Bytes;

    use super::*;
    use crate::{
        cookie::Cookie,
        http::header::{HeaderValue, CONTENT_TYPE},
        test::TestRequest,
    };

    #[actix_rt::test]
    async fn customize_responder() {
        let req = TestRequest::default().to_http_request();
        let res = "test"
            .to_string()
            .customize()
            .with_status(StatusCode::BAD_REQUEST)
            .respond_to(&req);

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = "test"
            .to_string()
            .customize()
            .insert_header(("content-type", "json"))
            .respond_to(&req);

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("json")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let res = "test"
            .to_string()
            .customize()
            .add_cookie(&Cookie::new("name", "value"))
            .respond_to(&req);

        assert!(res.status().is_success());
        assert_eq!(
            res.cookies().collect::<Vec<Cookie<'_>>>(),
            vec![Cookie::new("name", "value")],
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );
    }

    #[actix_rt::test]
    async fn tuple_responder_with_status_code() {
        let req = TestRequest::default().to_http_request();
        let res = ("test".to_string(), StatusCode::BAD_REQUEST).respond_to(&req);
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );

        let req = TestRequest::default().to_http_request();
        let res = ("test".to_string(), StatusCode::OK)
            .customize()
            .insert_header((CONTENT_TYPE, mime::APPLICATION_JSON))
            .respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/json")
        );
        assert_eq!(
            to_bytes(res.into_body()).await.unwrap(),
            Bytes::from_static(b"test"),
        );
    }
}
