use std::convert::TryFrom;
use std::net;
use std::rc::Rc;
use std::time::Duration;

use bytes::Bytes;
use futures_core::Stream;
use serde::Serialize;

use actix_http::body::Body;
use actix_http::http::header::IntoHeaderValue;
use actix_http::http::{Error as HttpError, HeaderMap, HeaderName, Method, Uri};
use actix_http::{Error, RequestHead};

use crate::sender::{RequestSender, SendClientRequest};
use crate::ClientConfig;

/// `FrozenClientRequest` struct represents clonable client request.
/// It could be used to send same request multiple times.
#[derive(Clone)]
pub struct FrozenClientRequest {
    pub(crate) head: Rc<RequestHead>,
    pub(crate) addr: Option<net::SocketAddr>,
    pub(crate) response_decompress: bool,
    pub(crate) timeout: Option<Duration>,
    pub(crate) config: Rc<ClientConfig>,
}

impl FrozenClientRequest {
    /// Get HTTP URI of request
    pub fn get_uri(&self) -> &Uri {
        &self.head.uri
    }

    /// Get HTTP method of this request
    pub fn get_method(&self) -> &Method {
        &self.head.method
    }

    /// Returns request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    /// Send a body.
    pub fn send_body<B>(&self, body: B) -> SendClientRequest
    where
        B: Into<Body>,
    {
        RequestSender::Rc(self.head.clone(), None).send_body(
            self.addr,
            self.response_decompress,
            self.timeout,
            self.config.as_ref(),
            body,
        )
    }

    /// Send a json body.
    pub fn send_json<T: Serialize>(&self, value: &T) -> SendClientRequest {
        RequestSender::Rc(self.head.clone(), None).send_json(
            self.addr,
            self.response_decompress,
            self.timeout,
            self.config.as_ref(),
            value,
        )
    }

    /// Send an urlencoded body.
    pub fn send_form<T: Serialize>(&self, value: &T) -> SendClientRequest {
        RequestSender::Rc(self.head.clone(), None).send_form(
            self.addr,
            self.response_decompress,
            self.timeout,
            self.config.as_ref(),
            value,
        )
    }

    /// Send a streaming body.
    pub fn send_stream<S, E>(&self, stream: S) -> SendClientRequest
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + 'static,
        E: Into<Error> + 'static,
    {
        RequestSender::Rc(self.head.clone(), None).send_stream(
            self.addr,
            self.response_decompress,
            self.timeout,
            self.config.as_ref(),
            stream,
        )
    }

    /// Send an empty body.
    pub fn send(&self) -> SendClientRequest {
        RequestSender::Rc(self.head.clone(), None).send(
            self.addr,
            self.response_decompress,
            self.timeout,
            self.config.as_ref(),
        )
    }

    /// Create a `FrozenSendBuilder` with extra headers
    pub fn extra_headers(&self, extra_headers: HeaderMap) -> FrozenSendBuilder {
        FrozenSendBuilder::new(self.clone(), extra_headers)
    }

    /// Create a `FrozenSendBuilder` with an extra header
    pub fn extra_header<K, V>(&self, key: K, value: V) -> FrozenSendBuilder
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        self.extra_headers(HeaderMap::new())
            .extra_header(key, value)
    }
}

/// Builder that allows to modify extra headers.
pub struct FrozenSendBuilder {
    req: FrozenClientRequest,
    extra_headers: HeaderMap,
    err: Option<HttpError>,
}

impl FrozenSendBuilder {
    pub(crate) fn new(req: FrozenClientRequest, extra_headers: HeaderMap) -> Self {
        Self {
            req,
            extra_headers,
            err: None,
        }
    }

    /// Insert a header, it overrides existing header in `FrozenClientRequest`.
    pub fn extra_header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match value.try_into() {
                Ok(value) => self.extra_headers.insert(key, value),
                Err(e) => self.err = Some(e.into()),
            },
            Err(e) => self.err = Some(e.into()),
        }
        self
    }

    /// Complete request construction and send a body.
    pub fn send_body<B>(self, body: B) -> SendClientRequest
    where
        B: Into<Body>,
    {
        if let Some(e) = self.err {
            return e.into();
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers)).send_body(
            self.req.addr,
            self.req.response_decompress,
            self.req.timeout,
            self.req.config.as_ref(),
            body,
        )
    }

    /// Complete request construction and send a json body.
    pub fn send_json<T: Serialize>(self, value: &T) -> SendClientRequest {
        if let Some(e) = self.err {
            return e.into();
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers)).send_json(
            self.req.addr,
            self.req.response_decompress,
            self.req.timeout,
            self.req.config.as_ref(),
            value,
        )
    }

    /// Complete request construction and send an urlencoded body.
    pub fn send_form<T: Serialize>(self, value: &T) -> SendClientRequest {
        if let Some(e) = self.err {
            return e.into();
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers)).send_form(
            self.req.addr,
            self.req.response_decompress,
            self.req.timeout,
            self.req.config.as_ref(),
            value,
        )
    }

    /// Complete request construction and send a streaming body.
    pub fn send_stream<S, E>(self, stream: S) -> SendClientRequest
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + 'static,
        E: Into<Error> + 'static,
    {
        if let Some(e) = self.err {
            return e.into();
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers)).send_stream(
            self.req.addr,
            self.req.response_decompress,
            self.req.timeout,
            self.req.config.as_ref(),
            stream,
        )
    }

    /// Complete request construction and send an empty body.
    pub fn send(self) -> SendClientRequest {
        if let Some(e) = self.err {
            return e.into();
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers)).send(
            self.req.addr,
            self.req.response_decompress,
            self.req.timeout,
            self.req.config.as_ref(),
        )
    }
}
