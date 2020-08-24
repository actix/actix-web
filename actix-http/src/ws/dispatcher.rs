use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_service::{IntoService, Service};
use actix_utils::dispatcher::{Dispatcher as InnerDispatcher, DispatcherError};

use super::{Codec, Frame, Message};

#[pin_project::pin_project]
pub struct Dispatcher<S, T>
where
    S: Service<Request = Frame, Response = Message> + 'static,
    T: AsyncRead + AsyncWrite,
{
    #[pin]
    inner: InnerDispatcher<S, T, Codec, Message>,
}

impl<S, T> Dispatcher<S, T>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Frame, Response = Message>,
    S::Future: 'static,
    S::Error: 'static,
{
    pub fn new<F: IntoService<S>>(io: T, service: F) -> Self {
        Dispatcher {
            inner: InnerDispatcher::new(Framed::new(io, Codec::new()), service),
        }
    }

    pub fn with<F: IntoService<S>>(framed: Framed<T, Codec>, service: F) -> Self {
        Dispatcher {
            inner: InnerDispatcher::new(framed, service),
        }
    }
}

impl<S, T> Future for Dispatcher<S, T>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Frame, Response = Message>,
    S::Future: 'static,
    S::Error: 'static,
{
    type Output = Result<(), DispatcherError<S::Error, Codec, Message>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx)
    }
}
