use std::{
    future::Future,
    net,
    pin::Pin,
    task::{Context, Poll},
};

use actix_codec::Framed;
use actix_http::{
    body::Body,
    client::{
        Connect as ClientConnect, ConnectError, Connection, ConnectionIo, SendRequestError,
    },
    h1::ClientCodec,
    Payload, RequestHead, RequestHeadType, ResponseHead,
};
use actix_service::Service;
use futures_core::{future::LocalBoxFuture, ready};

use crate::response::ClientResponse;

pub type ConnectorService = Box<
    dyn Service<
        ConnectRequest,
        Response = ConnectResponse,
        Error = SendRequestError,
        Future = LocalBoxFuture<'static, Result<ConnectResponse, SendRequestError>>,
    >,
>;

pub enum ConnectRequest {
    Client(RequestHeadType, Body, Option<net::SocketAddr>),
    Tunnel(RequestHead, Option<net::SocketAddr>),
}

pub enum ConnectResponse {
    Client(ClientResponse),
    Tunnel(ResponseHead, Framed<BoxedSocket, ClientCodec>),
}

impl ConnectResponse {
    pub fn into_client_response(self) -> ClientResponse {
        match self {
            ConnectResponse::Client(res) => res,
            _ => panic!(
                "ClientResponse only reachable with ConnectResponse::ClientResponse variant"
            ),
        }
    }

    pub fn into_tunnel_response(self) -> (ResponseHead, Framed<BoxedSocket, ClientCodec>) {
        match self {
            ConnectResponse::Tunnel(head, framed) => (head, framed),
            _ => panic!(
                "TunnelResponse only reachable with ConnectResponse::TunnelResponse variant"
            ),
        }
    }
}

pub(crate) struct DefaultConnector<S> {
    connector: S,
}

impl<S> DefaultConnector<S> {
    pub(crate) fn new(connector: S) -> Self {
        Self { connector }
    }
}

impl<S> Service<ConnectRequest> for DefaultConnector<S>
where
    S: Service<ClientConnect, Error = ConnectError>,
    S::Response: Connection,
    <S::Response as Connection>::Io: 'static,
{
    type Response = ConnectResponse;
    type Error = SendRequestError;
    type Future = ConnectRequestFuture<S::Future, <S::Response as Connection>::Io>;

    actix_service::forward_ready!(connector);

    fn call(&self, req: ConnectRequest) -> Self::Future {
        // connect to the host
        let fut = match req {
            ConnectRequest::Client(ref head, .., addr) => self.connector.call(ClientConnect {
                uri: head.as_ref().uri.clone(),
                addr,
            }),
            ConnectRequest::Tunnel(ref head, addr) => self.connector.call(ClientConnect {
                uri: head.uri.clone(),
                addr,
            }),
        };

        ConnectRequestFuture::Connection {
            fut,
            req: Some(req),
        }
    }
}

pin_project_lite::pin_project! {
    #[project = ConnectRequestProj]
    pub(crate) enum ConnectRequestFuture<Fut, Io> {
        Connection {
            #[pin]
            fut: Fut,
            req: Option<ConnectRequest>
        },
        Client {
            fut: LocalBoxFuture<'static, Result<(ResponseHead, Payload), SendRequestError>>
        },
        Tunnel {
            fut: LocalBoxFuture<
                'static,
                Result<(ResponseHead, Framed<Io, ClientCodec>), SendRequestError>,
            >,
        }
    }
}

impl<Fut, C, Io> Future for ConnectRequestFuture<Fut, Io>
where
    Fut: Future<Output = Result<C, ConnectError>>,
    C: Connection<Io = Io>,
    Io: ConnectionIo,
{
    type Output = Result<ConnectResponse, SendRequestError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project() {
            ConnectRequestProj::Connection { fut, req } => {
                let connection = ready!(fut.poll(cx))?;
                let req = req.take().unwrap();
                match req {
                    ConnectRequest::Client(head, body, ..) => {
                        // send request
                        let fut = ConnectRequestFuture::Client {
                            fut: connection.send_request(head, body),
                        };
                        self.set(fut);
                    }
                    ConnectRequest::Tunnel(head, ..) => {
                        // send request
                        let fut = ConnectRequestFuture::Tunnel {
                            fut: connection.open_tunnel(RequestHeadType::from(head)),
                        };
                        self.set(fut);
                    }
                }
                self.poll(cx)
            }
            ConnectRequestProj::Client { fut } => {
                let (head, payload) = ready!(fut.as_mut().poll(cx))?;
                Poll::Ready(Ok(ConnectResponse::Client(ClientResponse::new(
                    head, payload,
                ))))
            }
            ConnectRequestProj::Tunnel { fut } => {
                let (head, framed) = ready!(fut.as_mut().poll(cx))?;
                let framed = framed.into_map_io(|io| Box::new(io) as _);
                Poll::Ready(Ok(ConnectResponse::Tunnel(head, framed)))
            }
        }
    }
}

pub type BoxedSocket = Box<dyn ConnectionIo>;
