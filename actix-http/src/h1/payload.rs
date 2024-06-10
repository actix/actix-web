//! Payload stream

use std::{
    cell::RefCell,
    collections::VecDeque,
    pin::Pin,
    rc::{Rc, Weak},
    task::{Context, Poll, Waker},
};

use bytes::Bytes;
use futures_core::Stream;

use crate::error::PayloadError;

/// max buffer size 32k
pub(crate) const MAX_BUFFER_SIZE: usize = 32_768;

#[derive(Debug, PartialEq, Eq)]
pub enum PayloadStatus {
    Read,
    Pause,
    Dropped,
}

/// Buffered stream of bytes chunks
///
/// Payload stores chunks in a vector. First chunk can be received with `poll_next`. Payload does
/// not notify current task when new data is available.
///
/// Payload can be used as `Response` body stream.
#[derive(Debug)]
pub struct Payload {
    inner: Rc<RefCell<Inner>>,
}

impl Payload {
    /// Creates a payload stream.
    ///
    /// This method construct two objects responsible for bytes stream generation:
    /// - `PayloadSender` - *Sender* side of the stream
    /// - `Payload` - *Receiver* side of the stream
    pub fn create(eof: bool) -> (PayloadSender, Payload) {
        let shared = Rc::new(RefCell::new(Inner::new(eof)));

        (
            PayloadSender::new(Rc::downgrade(&shared)),
            Payload { inner: shared },
        )
    }

    /// Creates an empty payload.
    pub(crate) fn empty() -> Payload {
        Payload {
            inner: Rc::new(RefCell::new(Inner::new(true))),
        }
    }

    /// Length of the data in this payload
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    /// Is payload empty
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().len() == 0
    }

    /// Put unused data back to payload
    #[inline]
    pub fn unread_data(&mut self, data: Bytes) {
        self.inner.borrow_mut().unread_data(data);
    }
}

impl Stream for Payload {
    type Item = Result<Bytes, PayloadError>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, PayloadError>>> {
        Pin::new(&mut *self.inner.borrow_mut()).poll_next(cx)
    }
}

/// Sender part of the payload stream
pub struct PayloadSender {
    inner: Weak<RefCell<Inner>>,
}

impl PayloadSender {
    fn new(inner: Weak<RefCell<Inner>>) -> Self {
        Self { inner }
    }

    #[inline]
    pub fn set_error(&mut self, err: PayloadError) {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow_mut().set_error(err)
        }
    }

    #[inline]
    pub fn feed_eof(&mut self) {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow_mut().feed_eof()
        }
    }

    #[inline]
    pub fn feed_data(&mut self, data: Bytes) {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow_mut().feed_data(data)
        }
    }

    #[allow(clippy::needless_pass_by_ref_mut)]
    #[inline]
    pub fn need_read(&self, cx: &mut Context<'_>) -> PayloadStatus {
        // we check need_read only if Payload (other side) is alive,
        // otherwise always return true (consume payload)
        if let Some(shared) = self.inner.upgrade() {
            if shared.borrow().need_read {
                PayloadStatus::Read
            } else {
                shared.borrow_mut().register_io(cx);
                PayloadStatus::Pause
            }
        } else {
            PayloadStatus::Dropped
        }
    }
}

#[derive(Debug)]
struct Inner {
    len: usize,
    eof: bool,
    err: Option<PayloadError>,
    need_read: bool,
    items: VecDeque<Bytes>,
    task: Option<Waker>,
    io_task: Option<Waker>,
}

impl Inner {
    fn new(eof: bool) -> Self {
        Inner {
            eof,
            len: 0,
            err: None,
            items: VecDeque::new(),
            need_read: true,
            task: None,
            io_task: None,
        }
    }

    /// Wake up future waiting for payload data to be available.
    fn wake(&mut self) {
        if let Some(waker) = self.task.take() {
            waker.wake();
        }
    }

    /// Wake up future feeding data to Payload.
    fn wake_io(&mut self) {
        if let Some(waker) = self.io_task.take() {
            waker.wake();
        }
    }

    /// Register future waiting data from payload.
    /// Waker would be used in `Inner::wake`
    fn register(&mut self, cx: &Context<'_>) {
        if self
            .task
            .as_ref()
            .map_or(true, |w| !cx.waker().will_wake(w))
        {
            self.task = Some(cx.waker().clone());
        }
    }

    // Register future feeding data to payload.
    /// Waker would be used in `Inner::wake_io`
    fn register_io(&mut self, cx: &Context<'_>) {
        if self
            .io_task
            .as_ref()
            .map_or(true, |w| !cx.waker().will_wake(w))
        {
            self.io_task = Some(cx.waker().clone());
        }
    }

    #[inline]
    fn set_error(&mut self, err: PayloadError) {
        self.err = Some(err);
    }

    #[inline]
    fn feed_eof(&mut self) {
        self.eof = true;
    }

    #[inline]
    fn feed_data(&mut self, data: Bytes) {
        self.len += data.len();
        self.items.push_back(data);
        self.need_read = self.len < MAX_BUFFER_SIZE;
        self.wake();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.len
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &Context<'_>,
    ) -> Poll<Option<Result<Bytes, PayloadError>>> {
        if let Some(data) = self.items.pop_front() {
            self.len -= data.len();
            self.need_read = self.len < MAX_BUFFER_SIZE;

            if self.need_read && !self.eof {
                self.register(cx);
            }
            self.wake_io();
            Poll::Ready(Some(Ok(data)))
        } else if let Some(err) = self.err.take() {
            Poll::Ready(Some(Err(err)))
        } else if self.eof {
            Poll::Ready(None)
        } else {
            self.need_read = true;
            self.register(cx);
            self.wake_io();
            Poll::Pending
        }
    }

    fn unread_data(&mut self, data: Bytes) {
        self.len += data.len();
        self.items.push_front(data);
    }
}

#[cfg(test)]
mod tests {
    use actix_utils::future::poll_fn;
    use static_assertions::{assert_impl_all, assert_not_impl_any};

    use super::*;

    assert_impl_all!(Payload: Unpin);
    assert_not_impl_any!(Payload: Send, Sync);

    assert_impl_all!(Inner: Unpin, Send, Sync);

    #[actix_rt::test]
    async fn test_unread_data() {
        let (_, mut payload) = Payload::create(false);

        payload.unread_data(Bytes::from("data"));
        assert!(!payload.is_empty());
        assert_eq!(payload.len(), 4);

        assert_eq!(
            Bytes::from("data"),
            poll_fn(|cx| Pin::new(&mut payload).poll_next(cx))
                .await
                .unwrap()
                .unwrap()
        );
    }
}
