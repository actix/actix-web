use std::{net, rc::Rc, time::Duration};

use actix_http::{
    body::MessageBody,
    error::HttpError,
    header::{HeaderMap, TryIntoHeaderPair},
    Method, RequestHead, Uri,
};
use bytes::Bytes;
use futures_core::Stream;
use serde::Serialize;

use crate::{
    client::ClientConfig,
    sender::{RequestSender, SendClientRequest},
    BoxError,
};

/// `FrozenClientRequest` struct represents cloneable client request.
///
/// It could be used to send same request multiple times.
#[derive(Clone)]
pub struct FrozenClientRequest {
    pub(crate) head: Rc<RequestHead>,
    pub(crate) addr: Option<net::SocketAddr>,
    pub(crate) response_decompress: bool,
    pub(crate) timeout: Option<Duration>,
    pub(crate) config: ClientConfig,
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
        B: MessageBody + 'static,
    {
        RequestSender::Rc(Rc::clone(&self.head), None).send_body(
            self.addr,
            self.response_decompress,
            self.timeout,
            &self.config,
            body,
        )
    }

    /// Send a json body.
    pub fn send_json<T: Serialize>(&self, value: &T) -> SendClientRequest {
        RequestSender::Rc(Rc::clone(&self.head), None).send_json(
            self.addr,
            self.response_decompress,
            self.timeout,
            &self.config,
            value,
        )
    }

    /// Send an urlencoded body.
    pub fn send_form<T: Serialize>(&self, value: &T) -> SendClientRequest {
        RequestSender::Rc(Rc::clone(&self.head), None).send_form(
            self.addr,
            self.response_decompress,
            self.timeout,
            &self.config,
            value,
        )
    }

    /// Send a streaming body.
    pub fn send_stream<S, E>(&self, stream: S) -> SendClientRequest
    where
        S: Stream<Item = Result<Bytes, E>> + 'static,
        E: Into<BoxError> + 'static,
    {
        RequestSender::Rc(Rc::clone(&self.head), None).send_stream(
            self.addr,
            self.response_decompress,
            self.timeout,
            &self.config,
            stream,
        )
    }

    /// Send an empty body.
    pub fn send(&self) -> SendClientRequest {
        RequestSender::Rc(Rc::clone(&self.head), None).send(
            self.addr,
            self.response_decompress,
            self.timeout,
            &self.config,
        )
    }

    /// Clones this `FrozenClientRequest`, returning a new one with extra headers added.
    pub fn extra_headers(&self, extra_headers: HeaderMap) -> FrozenSendBuilder {
        FrozenSendBuilder::new(self.clone(), extra_headers)
    }

    /// Clones this `FrozenClientRequest`, returning a new one with the extra header added.
    pub fn extra_header(&self, header: impl TryIntoHeaderPair) -> FrozenSendBuilder {
        self.extra_headers(HeaderMap::new()).extra_header(header)
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
    pub fn extra_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        match header.try_into_pair() {
            Ok((key, value)) => {
                self.extra_headers.insert(key, value);
            }

            Err(err) => self.err = Some(err.into()),
        }

        self
    }

    /// Complete request construction and send a body.
    pub fn send_body(self, body: impl MessageBody + 'static) -> SendClientRequest {
        if let Some(err) = self.err {
            return err.into();
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers)).send_body(
            self.req.addr,
            self.req.response_decompress,
            self.req.timeout,
            &self.req.config,
            body,
        )
    }

    /// Complete request construction and send a json body.
    pub fn send_json(self, value: impl Serialize) -> SendClientRequest {
        if let Some(err) = self.err {
            return err.into();
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers)).send_json(
            self.req.addr,
            self.req.response_decompress,
            self.req.timeout,
            &self.req.config,
            value,
        )
    }

    /// Complete request construction and send an urlencoded body.
    pub fn send_form(self, value: impl Serialize) -> SendClientRequest {
        if let Some(err) = self.err {
            return err.into();
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers)).send_form(
            self.req.addr,
            self.req.response_decompress,
            self.req.timeout,
            &self.req.config,
            value,
        )
    }

    /// Complete request construction and send a streaming body.
    pub fn send_stream<S, E>(self, stream: S) -> SendClientRequest
    where
        S: Stream<Item = Result<Bytes, E>> + 'static,
        E: Into<BoxError> + 'static,
    {
        if let Some(err) = self.err {
            return err.into();
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers)).send_stream(
            self.req.addr,
            self.req.response_decompress,
            self.req.timeout,
            &self.req.config,
            stream,
        )
    }

    /// Complete request construction and send an empty body.
    pub fn send(self) -> SendClientRequest {
        if let Some(err) = self.err {
            return err.into();
        }

        RequestSender::Rc(self.req.head, Some(self.extra_headers)).send(
            self.req.addr,
            self.req.response_decompress,
            self.req.timeout,
            &self.req.config,
        )
    }
}
