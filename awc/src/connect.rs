use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::{fmt, io, mem, net};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::body::Body;
use actix_http::client::{
    Connect as ClientConnect, ConnectError, Connection, SendRequestError,
};
use actix_http::h1::ClientCodec;
use actix_http::http::HeaderMap;
use actix_http::{RequestHead, RequestHeadType, ResponseHead};
use actix_service::Service;

use crate::response::ClientResponse;

pub(crate) struct ConnectorWrapper<T>(pub T);

pub(crate) trait Connect {
    fn send_request(
        &mut self,
        head: RequestHead,
        body: Body,
        addr: Option<net::SocketAddr>,
    ) -> Pin<Box<dyn Future<Output = Result<ClientResponse, SendRequestError>>>>;

    fn send_request_extra(
        &mut self,
        head: Rc<RequestHead>,
        extra_headers: Option<HeaderMap>,
        body: Body,
        addr: Option<net::SocketAddr>,
    ) -> Pin<Box<dyn Future<Output = Result<ClientResponse, SendRequestError>>>>;

    /// Send request, returns Response and Framed
    fn open_tunnel(
        &mut self,
        head: RequestHead,
        addr: Option<net::SocketAddr>,
    ) -> Pin<
        Box<
            dyn Future<
                Output = Result<
                    (ResponseHead, Framed<BoxedSocket, ClientCodec>),
                    SendRequestError,
                >,
            >,
        >,
    >;

    /// Send request and extra headers, returns Response and Framed
    fn open_tunnel_extra(
        &mut self,
        head: Rc<RequestHead>,
        extra_headers: Option<HeaderMap>,
        addr: Option<net::SocketAddr>,
    ) -> Pin<
        Box<
            dyn Future<
                Output = Result<
                    (ResponseHead, Framed<BoxedSocket, ClientCodec>),
                    SendRequestError,
                >,
            >,
        >,
    >;
}

impl<T> Connect for ConnectorWrapper<T>
where
    T: Service<Request = ClientConnect, Error = ConnectError>,
    T::Response: Connection,
    <T::Response as Connection>::Io: 'static,
    <T::Response as Connection>::Future: 'static,
    <T::Response as Connection>::TunnelFuture: 'static,
    T::Future: 'static,
{
    fn send_request(
        &mut self,
        head: RequestHead,
        body: Body,
        addr: Option<net::SocketAddr>,
    ) -> Pin<Box<dyn Future<Output = Result<ClientResponse, SendRequestError>>>> {
        // connect to the host
        let fut = self.0.call(ClientConnect {
            uri: head.uri.clone(),
            addr,
        });

        Box::pin(async move {
            let connection = fut.await?;

            // send request
            connection
                .send_request(RequestHeadType::from(head), body)
                .await
                .map(|(head, payload)| ClientResponse::new(head, payload))
        })
    }

    fn send_request_extra(
        &mut self,
        head: Rc<RequestHead>,
        extra_headers: Option<HeaderMap>,
        body: Body,
        addr: Option<net::SocketAddr>,
    ) -> Pin<Box<dyn Future<Output = Result<ClientResponse, SendRequestError>>>> {
        // connect to the host
        let fut = self.0.call(ClientConnect {
            uri: head.uri.clone(),
            addr,
        });

        Box::pin(async move {
            let connection = fut.await?;

            // send request
            let (head, payload) = connection
                .send_request(RequestHeadType::Rc(head, extra_headers), body)
                .await?;

            Ok(ClientResponse::new(head, payload))
        })
    }

    fn open_tunnel(
        &mut self,
        head: RequestHead,
        addr: Option<net::SocketAddr>,
    ) -> Pin<
        Box<
            dyn Future<
                Output = Result<
                    (ResponseHead, Framed<BoxedSocket, ClientCodec>),
                    SendRequestError,
                >,
            >,
        >,
    > {
        // connect to the host
        let fut = self.0.call(ClientConnect {
            uri: head.uri.clone(),
            addr,
        });

        Box::pin(async move {
            let connection = fut.await?;

            // send request
            let (head, framed) =
                connection.open_tunnel(RequestHeadType::from(head)).await?;

            let framed = framed.into_map_io(|io| BoxedSocket(Box::new(Socket(io))));
            Ok((head, framed))
        })
    }

    fn open_tunnel_extra(
        &mut self,
        head: Rc<RequestHead>,
        extra_headers: Option<HeaderMap>,
        addr: Option<net::SocketAddr>,
    ) -> Pin<
        Box<
            dyn Future<
                Output = Result<
                    (ResponseHead, Framed<BoxedSocket, ClientCodec>),
                    SendRequestError,
                >,
            >,
        >,
    > {
        // connect to the host
        let fut = self.0.call(ClientConnect {
            uri: head.uri.clone(),
            addr,
        });

        Box::pin(async move {
            let connection = fut.await?;

            // send request
            let (head, framed) = connection
                .open_tunnel(RequestHeadType::Rc(head, extra_headers))
                .await?;

            let framed = framed.into_map_io(|io| BoxedSocket(Box::new(Socket(io))));
            Ok((head, framed))
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
    unsafe fn prepare_uninitialized_buffer(
        &self,
        buf: &mut [mem::MaybeUninit<u8>],
    ) -> bool {
        self.0.as_read().prepare_uninitialized_buffer(buf)
    }

    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
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

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(self.get_mut().0.as_write()).poll_shutdown(cx)
    }
}
