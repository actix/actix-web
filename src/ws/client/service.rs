//! websockets client
use std::marker::PhantomData;

use actix_net::codec::Framed;
use actix_net::connector::{Connect as TcpConnect, ConnectorError, DefaultConnector};
use actix_net::service::Service;
use base64;
use futures::future::{err, Either, FutureResult};
use futures::{Async, Future, Poll, Sink, Stream};
use http::header::{self, HeaderValue};
use http::{HttpTryFrom, StatusCode};
use rand;
use sha1::Sha1;
use tokio_io::{AsyncRead, AsyncWrite};

use body::BodyLength;
use client::ClientResponse;
use h1;
use ws::Codec;

use super::{ClientError, Connect, Protocol};

/// Default client, uses default connector.
pub type DefaultClient = Client<DefaultConnector>;

/// WebSocket's client
pub struct Client<T>
where
    T: Service<TcpConnect, Error = ConnectorError>,
    T::Response: AsyncRead + AsyncWrite,
{
    connector: T,
}

impl<T> Client<T>
where
    T: Service<TcpConnect, Error = ConnectorError>,
    T::Response: AsyncRead + AsyncWrite,
{
    /// Create new websocket's client factory
    pub fn new(connector: T) -> Self {
        Client { connector }
    }
}

impl Default for Client<DefaultConnector> {
    fn default() -> Self {
        Client::new(DefaultConnector::default())
    }
}

impl<T> Clone for Client<T>
where
    T: Service<TcpConnect, Error = ConnectorError> + Clone,
    T::Response: AsyncRead + AsyncWrite,
{
    fn clone(&self) -> Self {
        Client {
            connector: self.connector.clone(),
        }
    }
}

impl<T> Service<Connect> for Client<T>
where
    T: Service<TcpConnect, Error = ConnectorError>,
    T::Response: AsyncRead + AsyncWrite + 'static,
    T::Future: 'static,
{
    type Response = Framed<T::Response, Codec>;
    type Error = ClientError;
    type Future = Either<
        FutureResult<Self::Response, Self::Error>,
        ClientResponseFut<T::Response>,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.connector.poll_ready().map_err(ClientError::from)
    }

    fn call(&mut self, mut req: Connect) -> Self::Future {
        if let Some(e) = req.err.take() {
            Either::A(err(e))
        } else if let Some(e) = req.http_err.take() {
            Either::A(err(e.into()))
        } else {
            // origin
            if let Some(origin) = req.origin.take() {
                req.request.set_header(header::ORIGIN, origin);
            }

            req.request.upgrade("websocket");
            req.request.set_header(header::SEC_WEBSOCKET_VERSION, "13");

            if let Some(protocols) = req.protocols.take() {
                req.request
                    .set_header(header::SEC_WEBSOCKET_PROTOCOL, protocols.as_str());
            }
            let mut request = match req.request.finish() {
                Ok(req) => req,
                Err(e) => return Either::A(err(e.into())),
            };

            if request.uri().host().is_none() {
                return Either::A(err(ClientError::InvalidUrl));
            }

            // supported protocols
            let proto = if let Some(scheme) = request.uri().scheme_part() {
                match Protocol::from(scheme.as_str()) {
                    Some(proto) => proto,
                    None => return Either::A(err(ClientError::InvalidUrl)),
                }
            } else {
                return Either::A(err(ClientError::InvalidUrl));
            };

            // Generate a random key for the `Sec-WebSocket-Key` header.
            // a base64-encoded (see Section 4 of [RFC4648]) value that,
            // when decoded, is 16 bytes in length (RFC 6455)
            let sec_key: [u8; 16] = rand::random();
            let key = base64::encode(&sec_key);

            request.headers_mut().insert(
                header::SEC_WEBSOCKET_KEY,
                HeaderValue::try_from(key.as_str()).unwrap(),
            );

            // prep connection
            let connect = TcpConnect::new(
                request.uri().host().unwrap(),
                request.uri().port().unwrap_or_else(|| proto.port()),
            );

            let fut = Box::new(
                self.connector
                    .call(connect)
                    .map_err(ClientError::from)
                    .and_then(move |io| {
                        // h1 protocol
                        let framed = Framed::new(io, h1::ClientCodec::default());
                        framed
                            .send((request.into_parts().0, BodyLength::None).into())
                            .map_err(ClientError::from)
                            .and_then(|framed| {
                                framed
                                    .into_future()
                                    .map_err(|(e, _)| ClientError::from(e))
                            })
                    }),
            );

            // start handshake
            Either::B(ClientResponseFut {
                key,
                fut,
                max_size: req.max_size,
                server_mode: req.server_mode,
                _t: PhantomData,
            })
        }
    }
}

/// Future that implementes client websocket handshake process.
///
/// It resolves to a `Framed<T, ws::Codec>` instance.
pub struct ClientResponseFut<T>
where
    T: AsyncRead + AsyncWrite,
{
    fut: Box<
        Future<
            Item = (Option<ClientResponse>, Framed<T, h1::ClientCodec>),
            Error = ClientError,
        >,
    >,
    key: String,
    max_size: usize,
    server_mode: bool,
    _t: PhantomData<T>,
}

impl<T> Future for ClientResponseFut<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Item = Framed<T, Codec>;
    type Error = ClientError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let (item, framed) = try_ready!(self.fut.poll());

        let res = match item {
            Some(res) => res,
            None => return Err(ClientError::Disconnected),
        };

        // verify response
        if res.status() != StatusCode::SWITCHING_PROTOCOLS {
            return Err(ClientError::InvalidResponseStatus(res.status()));
        }
        // Check for "UPGRADE" to websocket header
        let has_hdr = if let Some(hdr) = res.headers().get(header::UPGRADE) {
            if let Ok(s) = hdr.to_str() {
                s.to_lowercase().contains("websocket")
            } else {
                false
            }
        } else {
            false
        };
        if !has_hdr {
            trace!("Invalid upgrade header");
            return Err(ClientError::InvalidUpgradeHeader);
        }
        // Check for "CONNECTION" header
        if let Some(conn) = res.headers().get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                if !s.to_lowercase().contains("upgrade") {
                    trace!("Invalid connection header: {}", s);
                    return Err(ClientError::InvalidConnectionHeader(conn.clone()));
                }
            } else {
                trace!("Invalid connection header: {:?}", conn);
                return Err(ClientError::InvalidConnectionHeader(conn.clone()));
            }
        } else {
            trace!("Missing connection header");
            return Err(ClientError::MissingConnectionHeader);
        }

        if let Some(key) = res.headers().get(header::SEC_WEBSOCKET_ACCEPT) {
            // field is constructed by concatenating /key/
            // with the string "258EAFA5-E914-47DA-95CA-C5AB0DC85B11" (RFC 6455)
            const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
            let mut sha1 = Sha1::new();
            sha1.update(self.key.as_ref());
            sha1.update(WS_GUID);
            let encoded = base64::encode(&sha1.digest().bytes());
            if key.as_bytes() != encoded.as_bytes() {
                trace!(
                    "Invalid challenge response: expected: {} received: {:?}",
                    encoded,
                    key
                );
                return Err(ClientError::InvalidChallengeResponse(encoded, key.clone()));
            }
        } else {
            trace!("Missing SEC-WEBSOCKET-ACCEPT header");
            return Err(ClientError::MissingWebSocketAcceptHeader);
        };

        // websockets codec
        let codec = if self.server_mode {
            Codec::new().max_size(self.max_size)
        } else {
            Codec::new().max_size(self.max_size).client_mode()
        };

        Ok(Async::Ready(framed.into_framed(codec)))
    }
}
