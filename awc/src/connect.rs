use std::{
    fmt, io, net,
    pin::Pin,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, Framed, ReadBuf};
use actix_http::{
    body::Body,
    client::{Connect as ClientConnect, ConnectError, Connection, SendRequestError},
    h1::ClientCodec,
    RequestHead, RequestHeadType, ResponseHead,
};
use actix_service::Service;
use futures_core::future::LocalBoxFuture;

use crate::response::ClientResponse;

pub(crate) struct ConnectorWrapper<T> {
    connector: T,
}

impl<T> ConnectorWrapper<T> {
    pub(crate) fn new(connector: T) -> Self {
        Self { connector }
    }
}

pub type ConnectService = Box<
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

impl<T> Service<ConnectRequest> for ConnectorWrapper<T>
where
    T: Service<ClientConnect, Error = ConnectError>,
    T::Response: Connection,
    <T::Response as Connection>::Io: 'static,
    T::Future: 'static,
{
    type Response = ConnectResponse;
    type Error = SendRequestError;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

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

        Box::pin(async move {
            let connection = fut.await?;

            match req {
                ConnectRequest::Client(head, body, ..) => {
                    // send request
                    let (head, payload) = connection.send_request(head, body).await?;

                    Ok(ConnectResponse::Client(ClientResponse::new(head, payload)))
                }
                ConnectRequest::Tunnel(head, ..) => {
                    // send request
                    let (head, framed) =
                        connection.open_tunnel(RequestHeadType::from(head)).await?;

                    let framed = framed.into_map_io(|io| BoxedSocket(Box::new(Socket(io))));
                    Ok(ConnectResponse::Tunnel(head, framed))
                }
            }
        })
    }
}

trait AsyncSocket {
    fn as_read(&self) -> &(dyn AsyncRead + Unpin);
    fn as_read_mut(&mut self) -> &mut (dyn AsyncRead + Unpin);
    fn as_write(&mut self) -> &mut (dyn AsyncWrite + Unpin);
}

struct Socket<T: AsyncRead + AsyncWrite + Unpin>(T);

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncSocket for Socket<T> {
    fn as_read(&self) -> &(dyn AsyncRead + Unpin) {
        &self.0
    }
    fn as_read_mut(&mut self) -> &mut (dyn AsyncRead + Unpin) {
        &mut self.0
    }
    fn as_write(&mut self) -> &mut (dyn AsyncWrite + Unpin) {
        &mut self.0
    }
}

pub struct BoxedSocket(Box<dyn AsyncSocket>);

impl fmt::Debug for BoxedSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BoxedSocket")
    }
}

impl AsyncRead for BoxedSocket {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(self.get_mut().0.as_read_mut()).poll_read(cx, buf)
    }
}

impl AsyncWrite for BoxedSocket {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(self.get_mut().0.as_write()).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.get_mut().0.as_write()).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.get_mut().0.as_write()).poll_shutdown(cx)
    }
}
