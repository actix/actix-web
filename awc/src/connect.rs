use std::io;

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_http::body::Body;
use actix_http::client::{ConnectError, Connection, SendRequestError};
use actix_http::h1::ClientCodec;
use actix_http::{http, RequestHead, ResponseHead};
use actix_service::Service;
use futures::{Future, Poll};

use crate::response::ClientResponse;

pub(crate) struct ConnectorWrapper<T>(pub T);

pub(crate) trait Connect {
    fn send_request(
        &mut self,
        head: RequestHead,
        body: Body,
    ) -> Box<Future<Item = ClientResponse, Error = SendRequestError>>;

    /// Send request, returns Response and Framed
    fn open_tunnel(
        &mut self,
        head: RequestHead,
    ) -> Box<
        Future<
            Item = (ResponseHead, Framed<BoxedSocket, ClientCodec>),
            Error = SendRequestError,
        >,
    >;
}

impl<T> Connect for ConnectorWrapper<T>
where
    T: Service<Request = http::Uri, Error = ConnectError>,
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
    ) -> Box<Future<Item = ClientResponse, Error = SendRequestError>> {
        Box::new(
            self.0
                // connect to the host
                .call(head.uri.clone())
                .from_err()
                // send request
                .and_then(move |connection| connection.send_request(head, body))
                .map(|(head, payload)| ClientResponse::new(head, payload)),
        )
    }

    fn open_tunnel(
        &mut self,
        head: RequestHead,
    ) -> Box<
        Future<
            Item = (ResponseHead, Framed<BoxedSocket, ClientCodec>),
            Error = SendRequestError,
        >,
    > {
        Box::new(
            self.0
                // connect to the host
                .call(head.uri.clone())
                .from_err()
                // send request
                .and_then(move |connection| connection.open_tunnel(head))
                .map(|(head, framed)| {
                    let framed = framed.map_io(|io| BoxedSocket(Box::new(Socket(io))));
                    (head, framed)
                }),
        )
    }
}

trait AsyncSocket {
    fn as_read(&self) -> &AsyncRead;
    fn as_read_mut(&mut self) -> &mut AsyncRead;
    fn as_write(&mut self) -> &mut AsyncWrite;
}

struct Socket<T: AsyncRead + AsyncWrite>(T);

impl<T: AsyncRead + AsyncWrite> AsyncSocket for Socket<T> {
    fn as_read(&self) -> &AsyncRead {
        &self.0
    }
    fn as_read_mut(&mut self) -> &mut AsyncRead {
        &mut self.0
    }
    fn as_write(&mut self) -> &mut AsyncWrite {
        &mut self.0
    }
}

pub struct BoxedSocket(Box<dyn AsyncSocket>);

impl io::Read for BoxedSocket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.as_read_mut().read(buf)
    }
}

impl AsyncRead for BoxedSocket {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.0.as_read().prepare_uninitialized_buffer(buf)
    }
}

impl io::Write for BoxedSocket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.as_write().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.as_write().flush()
    }
}

impl AsyncWrite for BoxedSocket {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.0.as_write().shutdown()
    }
}
