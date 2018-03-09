//! Payload stream
use std::cmp;
use std::rc::{Rc, Weak};
use std::cell::RefCell;
use std::collections::VecDeque;
use bytes::{Bytes, BytesMut};
use futures::{Async, Poll, Stream};

use error::PayloadError;

#[derive(Debug, PartialEq)]
pub(crate) enum PayloadStatus {
    Read,
    Pause,
    Dropped,
}

/// Buffered stream of bytes chunks
///
/// Payload stores chunks in a vector. First chunk can be received with `.readany()` method.
/// Payload stream is not thread safe. Payload does not notify current task when
/// new data is available.
///
/// Payload stream can be used as `HttpResponse` body stream.
#[derive(Debug)]
pub struct Payload {
    inner: Rc<RefCell<Inner>>,
}

impl Payload {

    /// Create payload stream.
    ///
    /// This method construct two objects responsible for bytes stream generation.
    ///
    /// * `PayloadSender` - *Sender* side of the stream
    ///
    /// * `Payload` - *Receiver* side of the stream
    pub fn new(eof: bool) -> (PayloadSender, Payload) {
        let shared = Rc::new(RefCell::new(Inner::new(eof)));

        (PayloadSender{inner: Rc::downgrade(&shared)}, Payload{inner: shared})
    }

    /// Create empty payload
    #[doc(hidden)]
    pub fn empty() -> Payload {
        Payload{inner: Rc::new(RefCell::new(Inner::new(true)))}
    }

    /// Indicates EOF of payload
    #[inline]
    pub fn eof(&self) -> bool {
        self.inner.borrow().eof()
    }

    /// Length of the data in this payload
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    /// Is payload empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().len() == 0
    }

    /// Put unused data back to payload
    #[inline]
    pub fn unread_data(&mut self, data: Bytes) {
        self.inner.borrow_mut().unread_data(data);
    }

    #[cfg(test)]
    pub(crate) fn readall(&self) -> Option<Bytes> {
        self.inner.borrow_mut().readall()
    }
}

impl Stream for Payload {
    type Item = Bytes;
    type Error = PayloadError;

    #[inline]
    fn poll(&mut self) -> Poll<Option<Bytes>, PayloadError> {
        self.inner.borrow_mut().readany()
    }
}

impl Clone for Payload {
    fn clone(&self) -> Payload {
        Payload{inner: Rc::clone(&self.inner)}
    }
}

/// Payload writer interface.
pub(crate) trait PayloadWriter {

    /// Set stream error.
    fn set_error(&mut self, err: PayloadError);

    /// Write eof into a stream which closes reading side of a stream.
    fn feed_eof(&mut self);

    /// Feed bytes into a payload stream
    fn feed_data(&mut self, data: Bytes);

    /// Need read data
    fn need_read(&self) -> PayloadStatus;
}

/// Sender part of the payload stream
pub struct PayloadSender {
    inner: Weak<RefCell<Inner>>,
}

impl PayloadWriter for PayloadSender {

    #[inline]
    fn set_error(&mut self, err: PayloadError) {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow_mut().set_error(err)
        }
    }

    #[inline]
    fn feed_eof(&mut self) {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow_mut().feed_eof()
        }
    }

    #[inline]
    fn feed_data(&mut self, data: Bytes) {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow_mut().feed_data(data)
        }
    }

    #[inline]
    fn need_read(&self) -> PayloadStatus {
        // we check need_read only if Payload (other side) is alive,
        // otherwise always return true (consume payload)
        if let Some(shared) = self.inner.upgrade() {
            if shared.borrow().need_read {
                PayloadStatus::Read
            } else {
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
}

impl Inner {

    fn new(eof: bool) -> Self {
        Inner {
            eof,
            len: 0,
            err: None,
            items: VecDeque::new(),
            need_read: true,
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
        self.need_read = false;
        self.items.push_back(data);
    }

    #[inline]
    fn eof(&self) -> bool {
        self.items.is_empty() && self.eof
    }

    #[inline]
    fn len(&self) -> usize {
        self.len
    }

    #[cfg(test)]
    pub(crate) fn readall(&mut self) -> Option<Bytes> {
        let len = self.items.iter().map(|b| b.len()).sum();
        if len > 0 {
            let mut buf = BytesMut::with_capacity(len);
            for item in &self.items {
                buf.extend_from_slice(item);
            }
            self.items = VecDeque::new();
            self.len = 0;
            Some(buf.take().freeze())
        } else {
            self.need_read = true;
            None
        }
    }

    fn readany(&mut self) -> Poll<Option<Bytes>, PayloadError> {
        if let Some(data) = self.items.pop_front() {
            self.len -= data.len();
            Ok(Async::Ready(Some(data)))
        } else if let Some(err) = self.err.take() {
            Err(err)
        } else if self.eof {
            Ok(Async::Ready(None))
        } else {
            self.need_read = true;
            Ok(Async::NotReady)
        }
    }

    fn unread_data(&mut self, data: Bytes) {
        self.len += data.len();
        self.items.push_front(data);
    }
}

pub struct PayloadHelper<S> {
    len: usize,
    items: VecDeque<Bytes>,
    stream: S,
}

impl<S> PayloadHelper<S> where S: Stream<Item=Bytes, Error=PayloadError> {

    pub fn new(stream: S) -> Self {
        PayloadHelper {
            len: 0,
            items: VecDeque::new(),
            stream,
        }
    }

    #[inline]
    fn poll_stream(&mut self) -> Poll<bool, PayloadError> {
        self.stream.poll().map(|res| {
            match res {
                Async::Ready(Some(data)) => {
                    self.len += data.len();
                    self.items.push_back(data);
                    Async::Ready(true)
                },
                Async::Ready(None) => Async::Ready(false),
                Async::NotReady => Async::NotReady,
            }
        })
    }

    #[inline]
    pub fn readany(&mut self) -> Poll<Option<Bytes>, PayloadError> {
        if let Some(data) = self.items.pop_front() {
            self.len -= data.len();
            Ok(Async::Ready(Some(data)))
        } else {
            match self.poll_stream()? {
                Async::Ready(true) => self.readany(),
                Async::Ready(false) => Ok(Async::Ready(None)),
                Async::NotReady => Ok(Async::NotReady),
            }
        }
    }

    #[inline]
    pub fn can_read(&mut self, size: usize) -> Poll<Option<bool>, PayloadError> {
        if size <= self.len {
            Ok(Async::Ready(Some(true)))
        } else {
            match self.poll_stream()? {
                Async::Ready(true) => self.can_read(size),
                Async::Ready(false) => Ok(Async::Ready(None)),
                Async::NotReady => Ok(Async::NotReady),
            }
        }
    }

    #[inline]
    pub fn get_chunk(&mut self) -> Poll<Option<&[u8]>, PayloadError> {
        if self.items.is_empty() {
            match self.poll_stream()? {
                Async::Ready(true) => (),
                Async::Ready(false) => return Ok(Async::Ready(None)),
                Async::NotReady => return Ok(Async::NotReady),
            }
        }
        match self.items.front().map(|c| c.as_ref()) {
            Some(chunk) => Ok(Async::Ready(Some(chunk))),
            None => Ok(Async::NotReady),
        }
    }

    #[inline]
    pub fn read_exact(&mut self, size: usize) -> Poll<Option<Bytes>, PayloadError> {
        if size <= self.len {
            self.len -= size;
            let mut chunk = self.items.pop_front().unwrap();
            if size < chunk.len() {
                let buf = chunk.split_to(size);
                self.items.push_front(chunk);
                Ok(Async::Ready(Some(buf)))
            }
            else if size == chunk.len() {
                Ok(Async::Ready(Some(chunk)))
            }
            else {
                let mut buf = BytesMut::with_capacity(size);
                buf.extend_from_slice(&chunk);

                while buf.len() < size {
                    let mut chunk = self.items.pop_front().unwrap();
                    let rem = cmp::min(size - buf.len(), chunk.len());
                    buf.extend_from_slice(&chunk.split_to(rem));
                    if !chunk.is_empty() {
                        self.items.push_front(chunk);
                    }
                }
                Ok(Async::Ready(Some(buf.freeze())))
            }
        } else {
            match self.poll_stream()? {
                Async::Ready(true) => self.read_exact(size),
                Async::Ready(false) => Ok(Async::Ready(None)),
                Async::NotReady => Ok(Async::NotReady),
            }
        }
    }

    #[inline]
    pub fn drop_payload(&mut self, size: usize) {
        if size <= self.len {
            self.len -= size;

            let mut len = 0;
            while len < size {
                let mut chunk = self.items.pop_front().unwrap();
                let rem = cmp::min(size-len, chunk.len());
                len += rem;
                if rem < chunk.len() {
                    chunk.split_to(rem);
                    self.items.push_front(chunk);
                }
            }
        }
    }

    pub fn copy(&mut self, size: usize) -> Poll<Option<BytesMut>, PayloadError> {
        if size <= self.len {
            let mut buf = BytesMut::with_capacity(size);
            for chunk in &self.items {
                if buf.len() < size {
                    let rem = cmp::min(size - buf.len(), chunk.len());
                    buf.extend_from_slice(&chunk[..rem]);
                }
                if buf.len() == size {
                    return Ok(Async::Ready(Some(buf)))
                }
            }
        }

        match self.poll_stream()? {
            Async::Ready(true) => self.copy(size),
            Async::Ready(false) => Ok(Async::Ready(None)),
            Async::NotReady => Ok(Async::NotReady),
        }
    }

    pub fn read_until(&mut self, line: &[u8]) -> Poll<Option<Bytes>, PayloadError> {
        let mut idx = 0;
        let mut num = 0;
        let mut offset = 0;
        let mut found = false;
        let mut length = 0;

        for no in 0..self.items.len() {
            {
                let chunk = &self.items[no];
                for (pos, ch) in chunk.iter().enumerate() {
                    if *ch == line[idx] {
                        idx += 1;
                        if idx == line.len() {
                            num = no;
                            offset = pos+1;
                            length += pos+1;
                            found = true;
                            break;
                        }
                    } else {
                        idx = 0
                    }
                }
                if !found {
                    length += chunk.len()
                }
            }

            if found {
                let mut buf = BytesMut::with_capacity(length);
                if num > 0 {
                    for _ in 0..num {
                        buf.extend_from_slice(&self.items.pop_front().unwrap());
                    }
                }
                if offset > 0 {
                    let mut chunk = self.items.pop_front().unwrap();
                    buf.extend_from_slice(&chunk.split_to(offset));
                    if !chunk.is_empty() {
                        self.items.push_front(chunk)
                    }
                }
                self.len -= length;
                return Ok(Async::Ready(Some(buf.freeze())))
            }
        }

        match self.poll_stream()? {
            Async::Ready(true) => self.read_until(line),
            Async::Ready(false) => Ok(Async::Ready(None)),
            Async::NotReady => Ok(Async::NotReady),
        }
    }

    pub fn readline(&mut self) -> Poll<Option<Bytes>, PayloadError> {
        self.read_until(b"\n")
    }

    pub fn unread_data(&mut self, data: Bytes) {
        self.len += data.len();
        self.items.push_front(data);
    }

    #[allow(dead_code)]
    pub fn remaining(&mut self) -> Bytes {
        self.items.iter_mut()
            .fold(BytesMut::new(), |mut b, c| {
                b.extend_from_slice(c);
                b
            }).freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use failure::Fail;
    use futures::future::{lazy, result};
    use tokio_core::reactor::Core;

    #[test]
    fn test_error() {
        let err: PayloadError = io::Error::new(io::ErrorKind::Other, "ParseError").into();
        assert_eq!(format!("{}", err), "ParseError");
        assert_eq!(format!("{}", err.cause().unwrap()), "ParseError");

        let err = PayloadError::Incomplete;
        assert_eq!(format!("{}", err), "A payload reached EOF, but is not complete.");
    }

    #[test]
    fn test_basic() {
        Core::new().unwrap().run(lazy(|| {
            let (_, payload) = Payload::new(false);
            let mut payload = PayloadHelper::new(payload);

            assert_eq!(payload.len, 0);
            assert_eq!(Async::NotReady, payload.readany().ok().unwrap());

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_eof() {
        Core::new().unwrap().run(lazy(|| {
            let (mut sender, payload) = Payload::new(false);
            let mut payload = PayloadHelper::new(payload);

            assert_eq!(Async::NotReady, payload.readany().ok().unwrap());
            sender.feed_data(Bytes::from("data"));
            sender.feed_eof();

            assert_eq!(Async::Ready(Some(Bytes::from("data"))),
                       payload.readany().ok().unwrap());
            assert_eq!(payload.len, 0);
            assert_eq!(Async::Ready(None), payload.readany().ok().unwrap());

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_err() {
        Core::new().unwrap().run(lazy(|| {
            let (mut sender, payload) = Payload::new(false);
            let mut payload = PayloadHelper::new(payload);

            assert_eq!(Async::NotReady, payload.readany().ok().unwrap());

            sender.set_error(PayloadError::Incomplete);
            payload.readany().err().unwrap();
            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_readany() {
        Core::new().unwrap().run(lazy(|| {
            let (mut sender, payload) = Payload::new(false);
            let mut payload = PayloadHelper::new(payload);

            sender.feed_data(Bytes::from("line1"));
            sender.feed_data(Bytes::from("line2"));

            assert_eq!(Async::Ready(Some(Bytes::from("line1"))),
                       payload.readany().ok().unwrap());
            assert_eq!(payload.len, 0);

            assert_eq!(Async::Ready(Some(Bytes::from("line2"))),
                       payload.readany().ok().unwrap());
            assert_eq!(payload.len, 0);

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_readexactly() {
        Core::new().unwrap().run(lazy(|| {
            let (mut sender, payload) = Payload::new(false);
            let mut payload = PayloadHelper::new(payload);

            assert_eq!(Async::NotReady, payload.read_exact(2).ok().unwrap());

            sender.feed_data(Bytes::from("line1"));
            sender.feed_data(Bytes::from("line2"));

            assert_eq!(Async::Ready(Some(Bytes::from_static(b"li"))),
                       payload.read_exact(2).ok().unwrap());
            assert_eq!(payload.len, 3);

            assert_eq!(Async::Ready(Some(Bytes::from_static(b"ne1l"))),
                       payload.read_exact(4).ok().unwrap());
            assert_eq!(payload.len, 4);

            sender.set_error(PayloadError::Incomplete);
            payload.read_exact(10).err().unwrap();

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_readuntil() {
        Core::new().unwrap().run(lazy(|| {
            let (mut sender, payload) = Payload::new(false);
            let mut payload = PayloadHelper::new(payload);

            assert_eq!(Async::NotReady, payload.read_until(b"ne").ok().unwrap());

            sender.feed_data(Bytes::from("line1"));
            sender.feed_data(Bytes::from("line2"));

            assert_eq!(Async::Ready(Some(Bytes::from("line"))),
                       payload.read_until(b"ne").ok().unwrap());
            assert_eq!(payload.len, 1);

            assert_eq!(Async::Ready(Some(Bytes::from("1line2"))),
                       payload.read_until(b"2").ok().unwrap());
            assert_eq!(payload.len, 0);

            sender.set_error(PayloadError::Incomplete);
            payload.read_until(b"b").err().unwrap();

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }

    #[test]
    fn test_unread_data() {
        Core::new().unwrap().run(lazy(|| {
            let (_, mut payload) = Payload::new(false);

            payload.unread_data(Bytes::from("data"));
            assert!(!payload.is_empty());
            assert_eq!(payload.len(), 4);

            assert_eq!(Async::Ready(Some(Bytes::from("data"))),
                       payload.poll().ok().unwrap());

            let res: Result<(), ()> = Ok(());
            result(res)
        })).unwrap();
    }
}
