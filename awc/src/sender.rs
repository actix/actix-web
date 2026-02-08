use std::{
    future::Future,
    net,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
    time::Duration,
};

use actix_http::{
    body::{BodyStream, MessageBody},
    error::HttpError,
    header::{self, HeaderMap, HeaderName, TryIntoHeaderValue},
    RequestHead, RequestHeadType,
};
#[cfg(feature = "__compress")]
use actix_http::{encoding::Decoder, header::ContentEncoding, Payload};
use actix_rt::time::{sleep, Sleep};
use bytes::Bytes;
use derive_more::From;
use futures_core::Stream;
use serde::Serialize;

use crate::{
    any_body::AnyBody,
    client::ClientConfig,
    error::{FreezeRequestError, InvalidUrl, SendRequestError},
    BoxError, ClientResponse, ConnectRequest, ConnectResponse,
};

#[derive(Debug, From)]
pub(crate) enum PrepForSendingError {
    Url(InvalidUrl),
    Http(HttpError),
    Json(serde_json::Error),
    Form(serde_urlencoded::ser::Error),
}

impl From<PrepForSendingError> for FreezeRequestError {
    fn from(err: PrepForSendingError) -> FreezeRequestError {
        match err {
            PrepForSendingError::Url(err) => FreezeRequestError::Url(err),
            PrepForSendingError::Http(err) => FreezeRequestError::Http(err),
            PrepForSendingError::Json(err) => {
                FreezeRequestError::Custom(Box::new(err), Box::new("json serialization error"))
            }
            PrepForSendingError::Form(err) => {
                FreezeRequestError::Custom(Box::new(err), Box::new("form serialization error"))
            }
        }
    }
}

impl From<PrepForSendingError> for SendRequestError {
    fn from(err: PrepForSendingError) -> SendRequestError {
        match err {
            PrepForSendingError::Url(err) => SendRequestError::Url(err),
            PrepForSendingError::Http(err) => SendRequestError::Http(err),
            PrepForSendingError::Json(err) => {
                SendRequestError::Custom(Box::new(err), Box::new("json serialization error"))
            }
            PrepForSendingError::Form(err) => {
                SendRequestError::Custom(Box::new(err), Box::new("form serialization error"))
            }
        }
    }
}

/// Future that sends request's payload and resolves to a server response.
#[must_use = "futures do nothing unless polled"]
pub enum SendClientRequest {
    Fut(
        Pin<Box<dyn Future<Output = Result<ConnectResponse, SendRequestError>>>>,
        // FIXME: use a pinned Sleep instead of box.
        Option<Pin<Box<Sleep>>>,
        bool,
    ),
    Err(Option<SendRequestError>),
}

impl SendClientRequest {
    pub(crate) fn new(
        send: Pin<Box<dyn Future<Output = Result<ConnectResponse, SendRequestError>>>>,
        response_decompress: bool,
        timeout: Option<Duration>,
    ) -> SendClientRequest {
        let delay = timeout.map(|d| Box::pin(sleep(d)));
        SendClientRequest::Fut(send, delay, response_decompress)
    }
}

#[cfg(feature = "__compress")]
impl Future for SendClientRequest {
    type Output = Result<ClientResponse<Decoder<Payload>>, SendRequestError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match this {
            SendClientRequest::Fut(send, delay, response_decompress) => {
                if let Some(delay) = delay {
                    if delay.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(Err(SendRequestError::Timeout));
                    }
                }

                let res = futures_core::ready!(send.as_mut().poll(cx)).map(|res| {
                    res.into_client_response()
                        ._timeout(delay.take())
                        .map_body(|head, payload| {
                            if *response_decompress {
                                Payload::Stream {
                                    payload: Decoder::from_headers(payload, &head.headers),
                                }
                            } else {
                                Payload::Stream {
                                    payload: Decoder::new(payload, ContentEncoding::Identity),
                                }
                            }
                        })
                });

                Poll::Ready(res)
            }
            SendClientRequest::Err(ref mut err) => match err.take() {
                Some(err) => Poll::Ready(Err(err)),
                None => panic!("Attempting to call completed future"),
            },
        }
    }
}

#[cfg(not(feature = "__compress"))]
impl Future for SendClientRequest {
    type Output = Result<ClientResponse, SendRequestError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match this {
            SendClientRequest::Fut(send, delay, _) => {
                if let Some(delay) = delay {
                    if delay.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(Err(SendRequestError::Timeout));
                    }
                }
                send.as_mut()
                    .poll(cx)
                    .map_ok(|res| res.into_client_response()._timeout(delay.take()))
            }
            SendClientRequest::Err(ref mut err) => match err.take() {
                Some(err) => Poll::Ready(Err(err)),
                None => panic!("Attempting to call completed future"),
            },
        }
    }
}

impl From<SendRequestError> for SendClientRequest {
    fn from(err: SendRequestError) -> Self {
        SendClientRequest::Err(Some(err))
    }
}

impl From<HttpError> for SendClientRequest {
    fn from(err: HttpError) -> Self {
        SendClientRequest::Err(Some(err.into()))
    }
}

impl From<PrepForSendingError> for SendClientRequest {
    fn from(err: PrepForSendingError) -> Self {
        SendClientRequest::Err(Some(err.into()))
    }
}

#[derive(Debug)]
pub(crate) enum RequestSender {
    Owned(RequestHead),
    Rc(Rc<RequestHead>, Option<HeaderMap>),
}

impl RequestSender {
    pub(crate) fn send_body(
        self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
        body: impl MessageBody + 'static,
    ) -> SendClientRequest {
        let req = match self {
            RequestSender::Owned(head) => ConnectRequest::Client(
                RequestHeadType::Owned(head),
                AnyBody::from_message_body(body).into_boxed(),
                addr,
            ),
            RequestSender::Rc(head, extra_headers) => ConnectRequest::Client(
                RequestHeadType::Rc(head, extra_headers),
                AnyBody::from_message_body(body).into_boxed(),
                addr,
            ),
        };

        let fut = config.connector.call(req);

        SendClientRequest::new(fut, response_decompress, timeout.or(config.timeout))
    }

    pub(crate) fn send_json(
        mut self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
        value: impl Serialize,
    ) -> SendClientRequest {
        let body = match serde_json::to_string(&value) {
            Ok(body) => body,
            Err(err) => return PrepForSendingError::Json(err).into(),
        };

        if let Err(err) = self.set_header_if_none(header::CONTENT_TYPE, "application/json") {
            return err.into();
        }

        self.send_body(addr, response_decompress, timeout, config, body)
    }

    pub(crate) fn send_form(
        mut self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
        value: impl Serialize,
    ) -> SendClientRequest {
        let body = match serde_urlencoded::to_string(value) {
            Ok(body) => body,
            Err(err) => return PrepForSendingError::Form(err).into(),
        };

        // set content-type
        if let Err(err) =
            self.set_header_if_none(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        {
            return err.into();
        }

        self.send_body(addr, response_decompress, timeout, config, body)
    }

    pub(crate) fn send_stream<S, E>(
        self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
        stream: S,
    ) -> SendClientRequest
    where
        S: Stream<Item = Result<Bytes, E>> + 'static,
        E: Into<BoxError> + 'static,
    {
        self.send_body(
            addr,
            response_decompress,
            timeout,
            config,
            BodyStream::new(stream),
        )
    }

    pub(crate) fn send(
        self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
    ) -> SendClientRequest {
        self.send_body(addr, response_decompress, timeout, config, ())
    }

    fn set_header_if_none<V>(&mut self, key: HeaderName, value: V) -> Result<(), HttpError>
    where
        V: TryIntoHeaderValue,
    {
        match self {
            RequestSender::Owned(head) => {
                if !head.headers.contains_key(&key) {
                    match value.try_into_value() {
                        Ok(value) => {
                            head.headers.insert(key, value);
                        }
                        Err(err) => return Err(err.into()),
                    }
                }
            }
            RequestSender::Rc(head, extra_headers) => {
                if !head.headers.contains_key(&key)
                    && !extra_headers.iter().any(|h| h.contains_key(&key))
                {
                    match value.try_into_value() {
                        Ok(v) => {
                            let h = extra_headers.get_or_insert(HeaderMap::new());
                            h.insert(key, v)
                        }
                        Err(err) => return Err(err.into()),
                    };
                }
            }
        }

        Ok(())
    }
}
