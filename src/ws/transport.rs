use actix_net::codec::Framed;
use actix_net::framed::{FramedTransport, FramedTransportError};
use actix_net::service::{IntoService, Service};
use futures::{Future, Poll};
use tokio_io::{AsyncRead, AsyncWrite};

use super::{Codec, Message};

pub struct Transport<S, T>
where
    S: Service,
    T: AsyncRead + AsyncWrite,
{
    inner: FramedTransport<S, T, Codec>,
}

impl<S, T> Transport<S, T>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Message, Response = Message>,
    S::Future: 'static,
    S::Error: 'static,
{
    pub fn new<F: IntoService<S>>(io: T, service: F) -> Self {
        Transport {
            inner: FramedTransport::new(Framed::new(io, Codec::new()), service),
        }
    }

    pub fn with<F: IntoService<S>>(framed: Framed<T, Codec>, service: F) -> Self {
        Transport {
            inner: FramedTransport::new(framed, service),
        }
    }
}

impl<S, T> Future for Transport<S, T>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Message, Response = Message>,
    S::Future: 'static,
    S::Error: 'static,
{
    type Item = ();
    type Error = FramedTransportError<S::Error, Codec>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.inner.poll()
    }
}
