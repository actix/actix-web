use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_service::{IntoService, Service};
use actix_utils::framed::{FramedTransport, FramedTransportError};
use futures::{Future, Poll};

use super::{Codec, Frame, Message};

pub struct Transport<S, T>
where
    S: Service<Request = Frame, Response = Message> + 'static,
    T: AsyncRead + AsyncWrite,
{
    inner: FramedTransport<S, T, Codec>,
}

impl<S, T> Transport<S, T>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Frame, Response = Message>,
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
    S: Service<Request = Frame, Response = Message>,
    S::Future: 'static,
    S::Error: 'static,
{
    type Item = ();
    type Error = FramedTransportError<S::Error, Codec>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.inner.poll()
    }
}
