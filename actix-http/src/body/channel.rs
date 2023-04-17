use bytes::Bytes;
use std::{error::Error as StdError, task::Poll};
use tokio::sync::mpsc::{error::SendError, UnboundedReceiver, UnboundedSender};

use super::{BodySize, MessageBody};
/// Returns a sender half and a receiver half that can be used as a body type.
///
/// # Examples
/// ```
/// use actix_web::{HttpResponse, web};
/// use std::convert::Infallible;
/// use actix_web::body::channel;
///
/// #[actix_rt::main]
/// async fn main() {
/// let (mut body_tx, body) = channel::<Infallible>();
///
/// let _ = web::block(move || {
///     body_tx.send(web::Bytes::from_static(b"body from another thread")).unwrap();
/// });
///
///
///  HttpResponse::Ok().body(body);
///  }
/// ```
pub fn channel<T: Into<Box<dyn StdError>>>() -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (Sender::new(tx), Receiver::new(rx))
}

/// Channel Sender wrapper
///
/// Senders can be cloned to create multiple senders that will send to the same underlying channel.
/// Senders should be mutable, as they can be used to close the channel.
#[derive(Debug)]
pub struct Sender<E> {
    tx: UnboundedSender<Result<Bytes, E>>,
}
impl<E> Sender<E> {
    pub fn new(tx: UnboundedSender<Result<Bytes, E>>) -> Self {
        Self { tx }
    }
    /// Submits a chunk of bytes to the response body stream.
    ///
    /// # Errors
    /// Errors if other side of channel body was dropped, returning `chunk`.
    pub fn send(&mut self, chunk: Bytes) -> Result<(), Bytes> {
        self.tx.send(Ok(chunk)).map_err(|SendError(err)| match err {
            Ok(chunk) => chunk,
            Err(_) => unreachable!(),
        })
    }

    /// Closes the stream, optionally sending an error.
    ///
    /// # Errors
    /// Errors if closing with error and other side of channel body was dropped, returning `error`.
    pub fn close(self, err: Option<E>) -> Result<(), E> {
        if let Some(err) = err {
            return self.tx.send(Err(err)).map_err(|SendError(err)| match err {
                Ok(_) => unreachable!(),
                Err(err) => err,
            });
        }
        Ok(())
    }
}

/// Clones the underlying [`UnboundedSender`].
/// This creates a new handle to the same channel, allowing a message to be sent on multiple
/// handles.
///
/// The returned [`Sender`] is a [`Clone`] of the original [`Sender`].
impl<E> Clone for Sender<E> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

/// Channel Receiver wrapper
#[derive(Debug)]
pub struct Receiver<E> {
    rx: UnboundedReceiver<Result<Bytes, E>>,
}

impl<E> Receiver<E> {
    pub fn new(rx: UnboundedReceiver<Result<Bytes, E>>) -> Self {
        Self { rx }
    }
}

impl<E> MessageBody for Receiver<E>
where
    E: Into<Box<dyn StdError>> + 'static,
{
    type Error = E;

    #[inline]
    fn size(&self) -> BodySize {
        BodySize::Stream
    }

    /// Attempts to pull out the next value of the underlying [`UnboundedReceiver`].
    /// If the underlying [`UnboundedReceiver`] is not ready, the current task is scheduled to
    /// receive a notification when it is ready to make progress.
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        self.rx.poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {

    use actix_rt::pin;
    use std::io;

    use super::*;
    use actix_utils::future::poll_fn;
    static_assertions::assert_impl_all!(Sender<io::Error>: Send, Sync, Unpin);
    static_assertions::assert_impl_all!(Receiver<io::Error>: Send, Sync, Unpin, MessageBody);

    #[actix_rt::test]
    async fn test_body_channel() {
        let (mut tx, rx) = channel::<io::Error>();
        let mut tx_cloned = tx.clone();
        let rx = rx.boxed();
        pin!(rx);

        assert_eq!(rx.size(), BodySize::Stream);

        tx.send(Bytes::from_static(b"test")).unwrap();
        tx_cloned.send(Bytes::from_static(b"test2")).unwrap();
        tx.close(None).unwrap();

        assert_eq!(
            poll_fn(|cx| rx.as_mut().poll_next(cx)).await.unwrap().ok(),
            Some(Bytes::from_static(b"test"))
        );

        assert_eq!(
            poll_fn(|cx| rx.as_mut().poll_next(cx)).await.unwrap().ok(),
            Some(Bytes::from_static(b"test2"))
        );
    }

    #[actix_rt::test]
    async fn test_body_channel_error() {
        let (mut tx, rx) = channel::<io::Error>();
        let mut tx_cloned = tx.clone();
        let rx = rx.boxed();
        pin!(rx);

        assert_eq!(rx.size(), BodySize::Stream);

        tx.send(Bytes::from_static(b"test")).unwrap();
        tx_cloned.send(Bytes::from_static(b"test2")).unwrap();

        let err = io::Error::new(io::ErrorKind::Other, "test");

        tx.close(Some(err)).unwrap();

        assert_eq!(
            poll_fn(|cx| rx.as_mut().poll_next(cx)).await.unwrap().ok(),
            Some(Bytes::from_static(b"test"))
        );

        let err = poll_fn(|cx| rx.as_mut().poll_next(cx)).await.unwrap().err();
        assert!(err.is_some());
        assert_eq!(err.unwrap().to_string(), "test");
    }
}
