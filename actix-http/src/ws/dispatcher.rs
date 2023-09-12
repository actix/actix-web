use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_service::{IntoService, Service};
use pin_project_lite::pin_project;

use super::{Codec, Frame, Message};

pin_project! {
    pub struct Dispatcher<S, T>
    where
        S: Service<Frame, Response = Message>,
        S: 'static,
        T: AsyncRead,
        T: AsyncWrite,
    {
        #[pin]
        inner: inner::Dispatcher<S, T, Codec, Message>,
    }
}

impl<S, T> Dispatcher<S, T>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Frame, Response = Message>,
    S::Future: 'static,
    S::Error: 'static,
{
    pub fn new<F: IntoService<S, Frame>>(io: T, service: F) -> Self {
        Dispatcher {
            inner: inner::Dispatcher::new(Framed::new(io, Codec::new()), service),
        }
    }

    pub fn with<F: IntoService<S, Frame>>(framed: Framed<T, Codec>, service: F) -> Self {
        Dispatcher {
            inner: inner::Dispatcher::new(framed, service),
        }
    }
}

impl<S, T> Future for Dispatcher<S, T>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Frame, Response = Message>,
    S::Future: 'static,
    S::Error: 'static,
{
    type Output = Result<(), inner::DispatcherError<S::Error, Codec, Message>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx)
    }
}

/// Framed dispatcher service and related utilities.
mod inner {
    // allow dead code since this mod was ripped from actix-utils
    #![allow(dead_code)]

    use core::{
        fmt,
        future::Future,
        mem,
        pin::Pin,
        task::{Context, Poll},
    };

    use actix_codec::Framed;
    use actix_service::{IntoService, Service};
    use futures_core::stream::Stream;
    use local_channel::mpsc;
    use pin_project_lite::pin_project;
    use tokio::io::{AsyncRead, AsyncWrite};
    use tokio_util::codec::{Decoder, Encoder};
    use tracing::debug;

    use crate::{body::BoxBody, Response};

    /// Framed transport errors
    pub enum DispatcherError<E, U, I>
    where
        U: Encoder<I> + Decoder,
    {
        /// Inner service error.
        Service(E),

        /// Frame encoding error.
        Encoder(<U as Encoder<I>>::Error),

        /// Frame decoding error.
        Decoder(<U as Decoder>::Error),
    }

    impl<E, U, I> From<E> for DispatcherError<E, U, I>
    where
        U: Encoder<I> + Decoder,
    {
        fn from(err: E) -> Self {
            DispatcherError::Service(err)
        }
    }

    impl<E, U, I> fmt::Debug for DispatcherError<E, U, I>
    where
        E: fmt::Debug,
        U: Encoder<I> + Decoder,
        <U as Encoder<I>>::Error: fmt::Debug,
        <U as Decoder>::Error: fmt::Debug,
    {
        fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
            match *self {
                DispatcherError::Service(ref e) => {
                    write!(fmt, "DispatcherError::Service({:?})", e)
                }
                DispatcherError::Encoder(ref e) => {
                    write!(fmt, "DispatcherError::Encoder({:?})", e)
                }
                DispatcherError::Decoder(ref e) => {
                    write!(fmt, "DispatcherError::Decoder({:?})", e)
                }
            }
        }
    }

    impl<E, U, I> fmt::Display for DispatcherError<E, U, I>
    where
        E: fmt::Display,
        U: Encoder<I> + Decoder,
        <U as Encoder<I>>::Error: fmt::Debug,
        <U as Decoder>::Error: fmt::Debug,
    {
        fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
            match *self {
                DispatcherError::Service(ref e) => write!(fmt, "{}", e),
                DispatcherError::Encoder(ref e) => write!(fmt, "{:?}", e),
                DispatcherError::Decoder(ref e) => write!(fmt, "{:?}", e),
            }
        }
    }

    impl<E, U, I> From<DispatcherError<E, U, I>> for Response<BoxBody>
    where
        E: fmt::Debug + fmt::Display,
        U: Encoder<I> + Decoder,
        <U as Encoder<I>>::Error: fmt::Debug,
        <U as Decoder>::Error: fmt::Debug,
    {
        fn from(err: DispatcherError<E, U, I>) -> Self {
            Response::internal_server_error().set_body(BoxBody::new(err.to_string()))
        }
    }

    /// Message type wrapper for signalling end of message stream.
    pub enum Message<T> {
        /// Message item.
        Item(T),

        /// Signal from service to flush all messages and stop processing.
        Close,
    }

    pin_project! {
        /// A future that reads frames from a [`Framed`] object and passes them to a [`Service`].
        pub struct Dispatcher<S, T, U, I>
        where
            S: Service<<U as Decoder>::Item, Response = I>,
            S::Error: 'static,
            S::Future: 'static,
            T: AsyncRead,
            T: AsyncWrite,
            U: Encoder<I>,
            U: Decoder,
            I: 'static,
            <U as Encoder<I>>::Error: fmt::Debug,
        {
            service: S,
            state: State<S, U, I>,
            #[pin]
            framed: Framed<T, U>,
            rx: mpsc::Receiver<Result<Message<I>, S::Error>>,
            tx: mpsc::Sender<Result<Message<I>, S::Error>>,
        }
    }

    enum State<S, U, I>
    where
        S: Service<<U as Decoder>::Item>,
        U: Encoder<I> + Decoder,
    {
        Processing,
        Error(DispatcherError<S::Error, U, I>),
        FramedError(DispatcherError<S::Error, U, I>),
        FlushAndStop,
        Stopping,
    }

    impl<S, U, I> State<S, U, I>
    where
        S: Service<<U as Decoder>::Item>,
        U: Encoder<I> + Decoder,
    {
        fn take_error(&mut self) -> DispatcherError<S::Error, U, I> {
            match mem::replace(self, State::Processing) {
                State::Error(err) => err,
                _ => panic!(),
            }
        }

        fn take_framed_error(&mut self) -> DispatcherError<S::Error, U, I> {
            match mem::replace(self, State::Processing) {
                State::FramedError(err) => err,
                _ => panic!(),
            }
        }
    }

    impl<S, T, U, I> Dispatcher<S, T, U, I>
    where
        S: Service<<U as Decoder>::Item, Response = I>,
        S::Error: 'static,
        S::Future: 'static,
        T: AsyncRead + AsyncWrite,
        U: Decoder + Encoder<I>,
        I: 'static,
        <U as Decoder>::Error: fmt::Debug,
        <U as Encoder<I>>::Error: fmt::Debug,
    {
        /// Create new `Dispatcher`.
        pub fn new<F>(framed: Framed<T, U>, service: F) -> Self
        where
            F: IntoService<S, <U as Decoder>::Item>,
        {
            let (tx, rx) = mpsc::channel();
            Dispatcher {
                framed,
                rx,
                tx,
                service: service.into_service(),
                state: State::Processing,
            }
        }

        /// Construct new `Dispatcher` instance with customer `mpsc::Receiver`
        pub fn with_rx<F>(
            framed: Framed<T, U>,
            service: F,
            rx: mpsc::Receiver<Result<Message<I>, S::Error>>,
        ) -> Self
        where
            F: IntoService<S, <U as Decoder>::Item>,
        {
            let tx = rx.sender();
            Dispatcher {
                framed,
                rx,
                tx,
                service: service.into_service(),
                state: State::Processing,
            }
        }

        /// Get sender handle.
        pub fn tx(&self) -> mpsc::Sender<Result<Message<I>, S::Error>> {
            self.tx.clone()
        }

        /// Get reference to a service wrapped by `Dispatcher` instance.
        pub fn service(&self) -> &S {
            &self.service
        }

        /// Get mutable reference to a service wrapped by `Dispatcher` instance.
        pub fn service_mut(&mut self) -> &mut S {
            &mut self.service
        }

        /// Get reference to a framed instance wrapped by `Dispatcher` instance.
        pub fn framed(&self) -> &Framed<T, U> {
            &self.framed
        }

        /// Get mutable reference to a framed instance wrapped by `Dispatcher` instance.
        pub fn framed_mut(&mut self) -> &mut Framed<T, U> {
            &mut self.framed
        }

        /// Read from framed object.
        fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> bool
        where
            S: Service<<U as Decoder>::Item, Response = I>,
            S::Error: 'static,
            S::Future: 'static,
            T: AsyncRead + AsyncWrite,
            U: Decoder + Encoder<I>,
            I: 'static,
            <U as Encoder<I>>::Error: fmt::Debug,
        {
            loop {
                let this = self.as_mut().project();
                match this.service.poll_ready(cx) {
                    Poll::Ready(Ok(_)) => {
                        let item = match this.framed.next_item(cx) {
                            Poll::Ready(Some(Ok(el))) => el,
                            Poll::Ready(Some(Err(err))) => {
                                *this.state = State::FramedError(DispatcherError::Decoder(err));
                                return true;
                            }
                            Poll::Pending => return false,
                            Poll::Ready(None) => {
                                *this.state = State::Stopping;
                                return true;
                            }
                        };

                        let tx = this.tx.clone();
                        let fut = this.service.call(item);
                        actix_rt::spawn(async move {
                            let item = fut.await;
                            let _ = tx.send(item.map(Message::Item));
                        });
                    }
                    Poll::Pending => return false,
                    Poll::Ready(Err(err)) => {
                        *this.state = State::Error(DispatcherError::Service(err));
                        return true;
                    }
                }
            }
        }

        /// Write to framed object.
        fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> bool
        where
            S: Service<<U as Decoder>::Item, Response = I>,
            S::Error: 'static,
            S::Future: 'static,
            T: AsyncRead + AsyncWrite,
            U: Decoder + Encoder<I>,
            I: 'static,
            <U as Encoder<I>>::Error: fmt::Debug,
        {
            loop {
                let mut this = self.as_mut().project();
                while !this.framed.is_write_buf_full() {
                    match Pin::new(&mut this.rx).poll_next(cx) {
                        Poll::Ready(Some(Ok(Message::Item(msg)))) => {
                            if let Err(err) = this.framed.as_mut().write(msg) {
                                *this.state = State::FramedError(DispatcherError::Encoder(err));
                                return true;
                            }
                        }
                        Poll::Ready(Some(Ok(Message::Close))) => {
                            *this.state = State::FlushAndStop;
                            return true;
                        }
                        Poll::Ready(Some(Err(err))) => {
                            *this.state = State::Error(DispatcherError::Service(err));
                            return true;
                        }
                        Poll::Ready(None) | Poll::Pending => break,
                    }
                }

                if !this.framed.is_write_buf_empty() {
                    match this.framed.flush(cx) {
                        Poll::Pending => break,
                        Poll::Ready(Ok(_)) => {}
                        Poll::Ready(Err(err)) => {
                            debug!("Error sending data: {:?}", err);
                            *this.state = State::FramedError(DispatcherError::Encoder(err));
                            return true;
                        }
                    }
                } else {
                    break;
                }
            }

            false
        }
    }

    impl<S, T, U, I> Future for Dispatcher<S, T, U, I>
    where
        S: Service<<U as Decoder>::Item, Response = I>,
        S::Error: 'static,
        S::Future: 'static,
        T: AsyncRead + AsyncWrite,
        U: Decoder + Encoder<I>,
        I: 'static,
        <U as Encoder<I>>::Error: fmt::Debug,
        <U as Decoder>::Error: fmt::Debug,
    {
        type Output = Result<(), DispatcherError<S::Error, U, I>>;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            loop {
                let this = self.as_mut().project();

                return match this.state {
                    State::Processing => {
                        if self.as_mut().poll_read(cx) || self.as_mut().poll_write(cx) {
                            continue;
                        } else {
                            Poll::Pending
                        }
                    }
                    State::Error(_) => {
                        // flush write buffer
                        if !this.framed.is_write_buf_empty() && this.framed.flush(cx).is_pending() {
                            return Poll::Pending;
                        }
                        Poll::Ready(Err(this.state.take_error()))
                    }
                    State::FlushAndStop => {
                        if !this.framed.is_write_buf_empty() {
                            this.framed.flush(cx).map(|res| {
                                if let Err(err) = res {
                                    debug!("Error sending data: {:?}", err);
                                }

                                Ok(())
                            })
                        } else {
                            Poll::Ready(Ok(()))
                        }
                    }
                    State::FramedError(_) => Poll::Ready(Err(this.state.take_framed_error())),
                    State::Stopping => Poll::Ready(Ok(())),
                };
            }
        }
    }
}
