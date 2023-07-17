use bytes::Bytes;
use std::{error::Error as StdError, task::Poll};
use tokio::sync::mpsc::{error::SendError, UnboundedReceiver, UnboundedSender};

use super::{BodySize, MessageBody};

/// Creates an unbounded mpsc (multi-producer, single-consumer) channel for communicating between asynchronous tasks.
///
/// This function returns a `Sender` half and a `Receiver` half that can be used as a body type, allowing for efficient streaming of data between tasks. The `Sender` can be cloned to allow sending to the same channel from multiple code locations, making it suitable for multi-producer scenarios. Only one `Receiver` is supported, adhering to the single-consumer principle.
///
/// Since the channel is unbounded, it does not provide backpressure support. This means that the `Sender` can keep sending data without waiting, even if the `Receiver` is not able to process it quickly enough, which may cause memory issues if not handled carefully.
///
/// If the `Receiver` is disconnected while trying to send, the `send` method will return a `SendError`. Similarly, if the `Sender` is disconnected while trying to receive, the data from the `Sender` will return `None`, indicating that there's no more data to be received.
///
/// This unbounded channel implementation is useful for streaming response bodies in web applications or other scenarios where it's essential to maintain a steady flow of data between asynchronous tasks. However, be cautious when using it in situations where the rate of incoming data may overwhelm the receiver, as it may lead to memory issues.
/// # Examples
/// ```
/// # use actix_web::{HttpResponse, web};
/// # use std::convert::Infallible;
/// # use actix_http::body::channel;
///
/// #[actix_rt::main]
/// async fn main() {
///     let (mut body_tx, body) = channel::<Infallible>();
///
///     let _ = web::block(move || {
///         body_tx.send(web::Bytes::from_static(b"body from another thread")).unwrap();
///     });
///
///     HttpResponse::Ok().body(body);
/// }
/// ```
pub fn channel<T: Into<Box<dyn StdError>>>() -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (Sender::new(tx), Receiver::new(rx))
}

/// Channel Sender\
/// Senders can be cloned to create multiple senders that will send to the same underlying channel.\
/// Senders should be mutable, as they can be used to close the channel.
#[derive(Debug)]
pub struct Sender<E> {
    tx: UnboundedSender<Result<Bytes, E>>,
}

impl<E> Sender<E> {
    /// Constructs a new instance of the Sender struct with the specified UnboundedSender.\
    /// UnboundedSender object representing the sender for underlying channel
    pub fn new(tx: UnboundedSender<Result<Bytes, E>>) -> Self {
        Self { tx }
    }
    /// Submits a chunk of bytes to the response body stream.
    pub fn send(&mut self, chunk: Bytes) -> Result<(), Bytes> {
        self.tx.send(Ok(chunk)).map_err(|SendError(err)| match err {
            Ok(chunk) => chunk,
            Err(_) => unreachable!(),
        })
    }

    /// Closes the stream, optionally sending an error.
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

/// Clones the underlying [`UnboundedSender`].\
/// This creates a new handle to the same channel, allowing a message to be sent on multiple
/// handles.\
///
/// The returned [`Sender`] is a [`Clone`] of the original [`Sender`].
impl<E> Clone for Sender<E> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

/// Channel Receiver\
/// Receivers are used to receive data from the underlying channel.\
/// Receivers should not be mutable, as they cannot be used to close the channel.\
/// Receivers can be used as a MessageBody, allowing for efficient streaming of data between tasks.\
/// Since the Receiver is a unbound stream, it does not provide backpressure support.
#[derive(Debug)]
pub struct Receiver<E> {
    rx: UnboundedReceiver<Result<Bytes, E>>,
}

impl<E> Receiver<E> {
    /// Constructs a new instance of the Receiver struct with the specified UnboundedReceiver
    pub fn new(rx: UnboundedReceiver<Result<Bytes, E>>) -> Self {
        Self { rx }
    }
}

/// Drop the underlying [`UnboundedReceiver`].\
/// This will cause the [`Receiver`] to stop receiving messages.
impl<E> Drop for Receiver<E> {
    fn drop(&mut self) {
        self.rx.close();
    }
}

impl<E> MessageBody for Receiver<E>
where
    E: Into<Box<dyn StdError>> + 'static,
{
    type Error = E;

    /// Returns the body size of the Receiver as a BodySize object.\
    /// Since the Receiver is a stream, the method returns BodySize::Stream
    #[inline]
    fn size(&self) -> BodySize {
        BodySize::Stream
    }

    /// Attempts to pull out the next value of the underlying [`UnboundedReceiver`].\
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
        let rx = rx.boxed();
        pin!(rx);

        assert_eq!(rx.size(), BodySize::Stream);

        tx.send(Bytes::from_static(b"test")).unwrap();

        tx.close(None).unwrap();

        assert_eq!(
            poll_fn(|cx| rx.as_mut().poll_next(cx)).await.unwrap().ok(),
            Some(Bytes::from_static(b"test"))
        );
    }

    #[actix_rt::test]
    async fn test_body_channel_error() {
        let (mut tx, rx) = channel::<io::Error>();
        let rx = rx.boxed();
        pin!(rx);

        assert_eq!(rx.size(), BodySize::Stream);

        tx.send(Bytes::from_static(b"test")).unwrap();

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

    #[actix_rt::test]
    async fn test_dropped_sender() {
        let (tx, rx) = channel::<io::Error>();
        let rx = rx.boxed();
        pin!(rx);
        drop(tx);
        let received = poll_fn(|cx| rx.as_mut().poll_next(cx)).await;
        assert!(received.is_none());
    }

    #[actix_rt::test]
    async fn test_dropped_receiver() {
        let (mut tx, rx) = channel::<io::Error>();
        let rx = rx.boxed();
        drop(rx);

        let err = tx.send(Bytes::from_static(b"test")).unwrap_err();
        assert_eq!(err, Bytes::from_static(b"test"));
    }

    #[actix_rt::test]
    async fn test_multiple_senders() {
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
    async fn test_backpressure() {
        let (mut tx, rx) = channel::<io::Error>();
        let mut tx_cloned = tx.clone();
        let rx = rx.boxed();
        pin!(rx);

        assert_eq!(rx.size(), BodySize::Stream);

        tx.send(Bytes::from_static(b"test")).unwrap();
        tx_cloned.send(Bytes::from_static(b"test2")).unwrap();

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
    async fn test_error_propagation() {
        let (mut tx, rx) = channel::<io::Error>();
        let mut tx_cloned = tx.clone();
        let rx = rx.boxed();
        pin!(rx);

        assert_eq!(rx.size(), BodySize::Stream);

        tx.send(Bytes::from_static(b"test")).unwrap();
        tx_cloned.send(Bytes::from_static(b"test2")).unwrap();

        let err = io::Error::new(io::ErrorKind::Other, "error");

        tx.close(Some(err)).unwrap();

        assert_eq!(
            poll_fn(|cx| rx.as_mut().poll_next(cx)).await.unwrap().ok(),
            Some(Bytes::from_static(b"test"))
        );
        assert_eq!(
            poll_fn(|cx| rx.as_mut().poll_next(cx)).await.unwrap().ok(),
            Some(Bytes::from_static(b"test2"))
        );
        let err = poll_fn(|cx| rx.as_mut().poll_next(cx)).await.unwrap().err();
        assert!(err.is_some());
        assert_eq!(err.unwrap().to_string(), "error");
    }
}
