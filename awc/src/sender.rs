use std::{
    future::Future,
    io, net,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
    time::Duration,
};

use actix_http::{
    body::{Body, BodyStream},
    http::{
        header::{self, HeaderMap, HeaderName, IntoHeaderValue},
        Error as HttpError,
    },
    Error, RequestHead, RequestHeadType,
};
use actix_rt::time::{sleep, Sleep};
use bytes::Bytes;
use derive_more::From;
use futures_core::Stream;
use serde::Serialize;

#[cfg(feature = "compress")]
use actix_http::{encoding::Decoder, http::header::ContentEncoding, Payload, PayloadStream};

use crate::connect::{ConnectRequest, ConnectResponse};
use crate::error::{FreezeRequestError, InvalidUrl, SendRequestError};
use crate::response::ClientResponse;
use crate::ClientConfig;

#[derive(Debug, From)]
pub(crate) enum PrepForSendingError {
    Url(InvalidUrl),
    Http(HttpError),
}

impl From<PrepForSendingError> for FreezeRequestError {
    fn from(err: PrepForSendingError) -> FreezeRequestError {
        match err {
            PrepForSendingError::Url(e) => FreezeRequestError::Url(e),
            PrepForSendingError::Http(e) => FreezeRequestError::Http(e),
        }
    }
}

impl From<PrepForSendingError> for SendRequestError {
    fn from(err: PrepForSendingError) -> SendRequestError {
        match err {
            PrepForSendingError::Url(e) => SendRequestError::Url(e),
            PrepForSendingError::Http(e) => SendRequestError::Http(e),
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

#[cfg(feature = "compress")]
impl Future for SendClientRequest {
    type Output = Result<ClientResponse<Decoder<Payload<PayloadStream>>>, SendRequestError>;

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
                    res.into_client_response()._timeout(delay.take()).map_body(
                        |head, payload| {
                            if *response_decompress {
                                Payload::Stream(Decoder::from_headers(payload, &head.headers))
                            } else {
                                Payload::Stream(Decoder::new(
                                    payload,
                                    ContentEncoding::Identity,
                                ))
                            }
                        },
                    )
                });

                Poll::Ready(res)
            }
            SendClientRequest::Err(ref mut e) => match e.take() {
                Some(e) => Poll::Ready(Err(e)),
                None => panic!("Attempting to call completed future"),
            },
        }
    }
}

#[cfg(not(feature = "compress"))]
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
            SendClientRequest::Err(ref mut e) => match e.take() {
                Some(e) => Poll::Ready(Err(e)),
                None => panic!("Attempting to call completed future"),
            },
        }
    }
}

impl From<SendRequestError> for SendClientRequest {
    fn from(e: SendRequestError) -> Self {
        SendClientRequest::Err(Some(e))
    }
}

impl From<Error> for SendClientRequest {
    fn from(e: Error) -> Self {
        SendClientRequest::Err(Some(e.into()))
    }
}

impl From<HttpError> for SendClientRequest {
    fn from(e: HttpError) -> Self {
        SendClientRequest::Err(Some(e.into()))
    }
}

impl From<PrepForSendingError> for SendClientRequest {
    fn from(e: PrepForSendingError) -> Self {
        SendClientRequest::Err(Some(e.into()))
    }
}

#[derive(Debug)]
pub(crate) enum RequestSender {
    Owned(RequestHead),
    Rc(Rc<RequestHead>, Option<HeaderMap>),
}

impl RequestSender {
    pub(crate) fn send_body<B>(
        self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
        body: B,
    ) -> SendClientRequest
    where
        B: Into<Body>,
    {
        let req = match self {
            RequestSender::Owned(head) => {
                ConnectRequest::Client(RequestHeadType::Owned(head), body.into(), addr)
            }
            RequestSender::Rc(head, extra_headers) => ConnectRequest::Client(
                RequestHeadType::Rc(head, extra_headers),
                body.into(),
                addr,
            ),
        };

        let fut = config.connector.call(req);

        SendClientRequest::new(fut, response_decompress, timeout.or(config.timeout))
    }

    pub(crate) fn send_json<T: Serialize>(
        mut self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
        value: &T,
    ) -> SendClientRequest {
        let body = match serde_json::to_string(value) {
            Ok(body) => body,
            // TODO: own error type
            Err(e) => return Error::from(io::Error::new(io::ErrorKind::Other, e)).into(),
        };

        if let Err(e) = self.set_header_if_none(header::CONTENT_TYPE, "application/json") {
            return e.into();
        }

        self.send_body(
            addr,
            response_decompress,
            timeout,
            config,
            Body::Bytes(Bytes::from(body)),
        )
    }

    pub(crate) fn send_form<T: Serialize>(
        mut self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
        value: &T,
    ) -> SendClientRequest {
        let body = match serde_urlencoded::to_string(value) {
            Ok(body) => body,
            // TODO: own error type
            Err(e) => return Error::from(io::Error::new(io::ErrorKind::Other, e)).into(),
        };

        // set content-type
        if let Err(e) =
            self.set_header_if_none(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        {
            return e.into();
        }

        self.send_body(
            addr,
            response_decompress,
            timeout,
            config,
            Body::Bytes(Bytes::from(body)),
        )
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
        S: Stream<Item = Result<Bytes, E>> + Unpin + 'static,
        E: Into<Error> + 'static,
    {
        self.send_body(
            addr,
            response_decompress,
            timeout,
            config,
            Body::from_message(BodyStream::new(stream)),
        )
    }

    pub(crate) fn send(
        self,
        addr: Option<net::SocketAddr>,
        response_decompress: bool,
        timeout: Option<Duration>,
        config: &ClientConfig,
    ) -> SendClientRequest {
        self.send_body(addr, response_decompress, timeout, config, Body::Empty)
    }

    fn set_header_if_none<V>(&mut self, key: HeaderName, value: V) -> Result<(), HttpError>
    where
        V: IntoHeaderValue,
    {
        match self {
            RequestSender::Owned(head) => {
                if !head.headers.contains_key(&key) {
                    match value.try_into_value() {
                        Ok(value) => {
                            head.headers.insert(key, value);
                        }
                        Err(e) => return Err(e.into()),
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
                        Err(e) => return Err(e.into()),
                    };
                }
            }
        }

        Ok(())
    }
}
