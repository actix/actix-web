use std::{
    error::Error as StdError,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use tokio::sync::mpsc::{error::SendError, UnboundedReceiver, UnboundedSender};

use super::{BodySize, MessageBody};

/// Creates an unbounded channel for streaming a body from another task or thread.
///
/// The body ends after all senders are dropped and buffered chunks are consumed.
/// There is no backpressure; a fast producer can exhaust memory.
///
/// # Examples
/// ```
/// use std::convert::Infallible;
/// use actix_http::{body, Response};
/// use bytes::Bytes;
///
/// let (mut sender, body) = body::channel::<Infallible>();
/// std::thread::spawn(move || {
///     let _ = sender.send(Bytes::from_static(b"hello"));
/// });
/// let response = Response::ok().set_body(body);
/// ```
pub fn channel<E: Into<Box<dyn StdError>>>() -> (Sender<E>, impl MessageBody<Error = E>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    (Sender { tx }, Receiver { rx })
}

/// Sends chunks to a channel body. Clone to add a producer.
#[derive(Debug)]
pub struct Sender<E> {
    tx: UnboundedSender<Result<Bytes, E>>,
}

impl<E> Clone for Sender<E> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<E> Sender<E> {
    /// Sends a body chunk. Empty chunks are skipped by the receiver.
    ///
    /// # Errors
    /// Returns the chunk if the body was dropped.
    pub fn send(&mut self, chunk: Bytes) -> Result<(), Bytes> {
        self.tx
            .send(Ok(chunk))
            .map_err(|SendError(item)| match item {
                Ok(chunk) => chunk,
                Err(_) => unreachable!(),
            })
    }

    /// Releases this sender.
    ///
    /// Other senders remain usable. The body ends after all senders are dropped
    /// and buffered chunks are consumed.
    pub fn close(self) {
        drop(self);
    }

    /// Queues an error after buffered chunks and releases this sender.
    ///
    /// Other senders remain usable until the body is dropped. A body error aborts
    /// transmission of the response.
    ///
    /// # Errors
    /// Returns the supplied error if the body was dropped.
    pub fn error(self, error: E) -> Result<(), E> {
        self.tx
            .send(Err(error))
            .map_err(|SendError(item)| match item {
                Err(error) => error,
                Ok(_) => unreachable!(),
            })
    }
}

#[derive(Debug)]
struct Receiver<E> {
    rx: UnboundedReceiver<Result<Bytes, E>>,
}

impl<E: Into<Box<dyn StdError>>> MessageBody for Receiver<E> {
    type Error = E;

    fn size(&self) -> BodySize {
        BodySize::Stream
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        loop {
            match std::task::ready!(self.rx.poll_recv(cx)) {
                Some(Ok(chunk)) if chunk.is_empty() => continue,
                item => return Poll::Ready(item),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io, pin::pin};

    use actix_utils::future::poll_fn;

    use super::*;

    static_assertions::assert_impl_all!(Sender<io::Error>: Clone, Send, Sync, Unpin);

    #[actix_rt::test]
    async fn delivers_chunks_before_error() {
        let (mut sender, body) = channel::<io::Error>();
        let mut other = sender.clone();
        let mut body = pin!(body);

        sender.send(Bytes::from_static(b"one")).unwrap();
        other.send(Bytes::from_static(b"two")).unwrap();
        sender.error(io::Error::other("failed")).unwrap();
        drop(other);

        for expected in [b"one", b"two"] {
            assert_eq!(
                poll_fn(|cx| body.as_mut().poll_next(cx))
                    .await
                    .unwrap()
                    .unwrap(),
                Bytes::from_static(expected)
            );
        }

        let error = poll_fn(|cx| body.as_mut().poll_next(cx))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error.to_string(), "failed");
        assert!(poll_fn(|cx| body.as_mut().poll_next(cx)).await.is_none());
    }

    #[test]
    fn closing_one_sender_keeps_other_senders_usable() {
        let (sender, body) = channel::<io::Error>();
        let mut other = sender.clone();
        let mut body = tokio_test::task::spawn(body);

        sender.close();
        assert!(body.enter(|cx, body| body.poll_next(cx)).is_pending());

        other
            .send(Bytes::from_static(b"remaining producer"))
            .unwrap();
        let chunk = tokio_test::assert_ready!(body.enter(|cx, body| body.poll_next(cx)));
        assert_eq!(chunk.unwrap().unwrap(), "remaining producer");

        other.close();
        assert!(tokio_test::assert_ready!(body.enter(|cx, body| body.poll_next(cx))).is_none());
    }

    #[test]
    fn dropped_receiver_returns_chunk_and_error() {
        let (mut sender, body) = channel::<io::Error>();
        drop(body);

        assert_eq!(
            sender.send(Bytes::from_static(b"unsent")).unwrap_err(),
            "unsent"
        );
        assert_eq!(
            sender
                .error(io::Error::other("unsent error"))
                .unwrap_err()
                .to_string(),
            "unsent error"
        );
    }

    #[test]
    fn send_from_another_thread_wakes_receiver() {
        let (mut sender, body) = channel::<io::Error>();
        let mut body = tokio_test::task::spawn(body);

        sender.send(Bytes::new()).unwrap();
        assert!(body.enter(|cx, body| body.poll_next(cx)).is_pending());

        let sender = std::thread::spawn(move || {
            sender.send(Bytes::from_static(b"from thread")).unwrap();
            sender
        })
        .join()
        .unwrap();

        assert!(body.is_woken());
        let chunk = tokio_test::assert_ready!(body.enter(|cx, body| body.poll_next(cx)));
        assert_eq!(chunk.unwrap().unwrap(), "from thread");

        assert!(body.enter(|cx, body| body.poll_next(cx)).is_pending());
        std::thread::spawn(move || drop(sender)).join().unwrap();

        assert!(body.is_woken());
        assert!(tokio_test::assert_ready!(body.enter(|cx, body| body.poll_next(cx))).is_none());
    }

    #[test]
    fn error_wakes_receiver_with_another_sender_alive() {
        let (sender, body) = channel::<io::Error>();
        let mut other = sender.clone();
        let mut body = tokio_test::task::spawn(body);

        assert!(body.enter(|cx, body| body.poll_next(cx)).is_pending());
        sender.error(io::Error::other("failed")).unwrap();

        assert!(body.is_woken());
        let error = tokio_test::assert_ready!(body.enter(|cx, body| body.poll_next(cx)));
        assert_eq!(error.unwrap().unwrap_err().to_string(), "failed");

        drop(body);
        assert!(other.send(Bytes::from_static(b"after error")).is_err());
    }

    #[actix_rt::test]
    async fn skips_empty_chunks() {
        let (mut sender, body) = channel::<io::Error>();
        let mut body = pin!(body);

        sender.send(Bytes::new()).unwrap();
        sender.send(Bytes::from_static(b"hello")).unwrap();
        sender.send(Bytes::new()).unwrap();
        sender.close();

        assert_eq!(body.size(), BodySize::Stream);
        assert_eq!(
            poll_fn(|cx| body.as_mut().poll_next(cx))
                .await
                .unwrap()
                .unwrap(),
            Bytes::from_static(b"hello")
        );
        assert!(poll_fn(|cx| body.as_mut().poll_next(cx)).await.is_none());
    }
}
