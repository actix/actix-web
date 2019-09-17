use std::net;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use derive_more::From;
use futures::{try_ready, Async, Future, Poll, Stream};
use serde::Serialize;
use serde_json;
use tokio_timer::Delay;

use actix_http::body::{Body, BodyStream};
use actix_http::encoding::Decoder;
use actix_http::http::header::{self, ContentEncoding, IntoHeaderValue};
use actix_http::http::{Error as HttpError, HeaderMap, HeaderName};
use actix_http::{Error, Payload, PayloadStream, RequestHead};

use crate::error::{FreezeRequestError, InvalidUrl, SendRequestError};
use crate::response::ClientResponse;
use crate::ClientConfig;

#[derive(Debug, From)]
pub(crate) enum PrepForSendingError {
    Url(InvalidUrl),
    Http(HttpError),
}

impl Into<FreezeRequestError> for PrepForSendingError {
    fn into(self) -> FreezeRequestError {
        match self {
            PrepForSendingError::Url(e) => FreezeRequestError::Url(e),
            PrepForSendingError::Http(e) => FreezeRequestError::Http(e),
        }
    }
}

impl Into<SendRequestError> for PrepForSendingError {
    fn into(self) -> SendRequestError {
        match self {
            PrepForSendingError::Url(e) => SendRequestError::Url(e),
            PrepForSendingError::Http(e) => SendRequestError::Http(e),
        }
    }
}

/// Future that sends request's payload and resolves to a server response.
#[must_use = "futures do nothing unless polled"]
pub enum SendClientRequest {
    Fut(
        Box<dyn Future<Item = ClientResponse, Error = SendRequestError>>,
        Option<Delay>,
        bool,
    ),
    Err(Option<SendRequestError>),
}

impl SendClientRequest {
    pub(crate) fn new(
        send: Box<dyn Future<Item = ClientResponse, Error = SendRequestError>>,
        response_decompress: bool,
        timeout: Option<Duration>,
    ) -> SendClientRequest {
        let delay = timeout.map(|t| Delay::new(Instant::now() + t));
        SendClientRequest::Fut(send, delay, response_decompress)
    }
}

impl Future for SendClientRequest {
    type Item = ClientResponse<Decoder<Payload<PayloadStream>>>;
    type Error = SendRequestError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self {
            SendClientRequest::Fut(send, delay, response_decompress) => {
                if delay.is_some() {
                    match delay.poll() {
                        Ok(Async::NotReady) => (),
                        _ => return Err(SendRequestError::Timeout),
                    }
                }

                let res = try_ready!(send.poll()).map_body(|head, payload| {
                    if *response_decompress {
                        Payload::Stream(Decoder::from_headers(payload, &head.headers))
                    } else {
                        Payload::Stream(Decoder::new(payload, ContentEncoding::Identity))
                    }
                });

                Ok(Async::Ready(res))
            }
            SendClientRequest::Err(ref mut e) => match e.take() {
                Some(e) => Err(e),
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
        let mut connector = config.connector.borrow_mut();

        let fut = match self {
            RequestSender::Owned(head) => {
                connector.send_request(head, body.into(), addr)
            }
            RequestSender::Rc(head, extra_headers) => {
                connector.send_request_extra(head, extra_headers, body.into(), addr)
            }
        };

        SendClientRequest::new(
            fut,
            response_decompress,
            timeout.or_else(|| config.timeout),
        )
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
            Err(e) => return Error::from(e).into(),
        };

        if let Err(e) = self.set_header_if_none(header::CONTENT_TYPE, "application/json")
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
            Err(e) => return Error::from(e).into(),
        };

        // set content-type
        if let Err(e) = self.set_header_if_none(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ) {
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
        S: Stream<Item = Bytes, Error = E> + 'static,
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

    fn set_header_if_none<V>(
        &mut self,
        key: HeaderName,
        value: V,
    ) -> Result<(), HttpError>
    where
        V: IntoHeaderValue,
    {
        match self {
            RequestSender::Owned(head) => {
                if !head.headers.contains_key(&key) {
                    match value.try_into() {
                        Ok(value) => head.headers.insert(key, value),
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            RequestSender::Rc(head, extra_headers) => {
                if !head.headers.contains_key(&key)
                    && !extra_headers.iter().any(|h| h.contains_key(&key))
                {
                    match value.try_into() {
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
