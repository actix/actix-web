use std::rc::{Rc, Weak};
use std::cell::RefCell;
use std::collections::VecDeque;
use bytes::Bytes;
use futures::{Async, Poll, Stream};
use futures::task::{Task, current as current_task};

pub type PayloadItem = Bytes;

const MAX_PAYLOAD_SIZE: usize = 65_536; // max buffer size 64k


/// Stream of byte chunks
///
/// Payload stores chunks in vector. First chunk can be received with `.readany()` method.
pub struct Payload {
    inner: Rc<RefCell<Inner>>,
}

impl Payload {

    pub(crate) fn new(eof: bool) -> (PayloadSender, Payload) {
        let shared = Rc::new(RefCell::new(Inner::new(eof)));

        (PayloadSender{inner: Rc::downgrade(&shared)},
         Payload{inner: shared})
    }

    /// Indicates paused state of the payload. If payload data is not consumed
    /// it get paused. Max size of not consumed data is 64k
    pub fn paused(&self) -> bool {
        self.inner.borrow().paused()
    }

    /// Indicates EOF of payload
    pub fn eof(&self) -> bool {
        self.inner.borrow().eof()
    }

    /// Length of the data in this payload
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    /// Is payload empty
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().len() == 0
    }

    /// Get any chunk of data
    pub fn readany(&mut self) -> Async<Option<PayloadItem>> {
        self.inner.borrow_mut().readany()
    }

    /// Put unused data back to payload
    pub fn unread_data(&mut self, data: PayloadItem) {
        self.inner.borrow_mut().unread_data(data);
    }
}


impl Stream for Payload {
    type Item = PayloadItem;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<PayloadItem>, ()> {
        Ok(self.readany())
    }
}

pub(crate) struct PayloadSender {
    inner: Weak<RefCell<Inner>>,
}

impl PayloadSender {
    pub(crate) fn feed_eof(&mut self) {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow_mut().feed_eof()
        }
    }

    pub(crate) fn feed_data(&mut self, data: Bytes) {
        if let Some(shared) = self.inner.upgrade() {
            shared.borrow_mut().feed_data(data)
        }
    }

    pub(crate) fn maybe_paused(&self) -> bool {
        match self.inner.upgrade() {
            Some(shared) => {
                let inner = shared.borrow();
                if inner.paused() && inner.len() < MAX_PAYLOAD_SIZE {
                    drop(inner);
                    shared.borrow_mut().resume();
                    false
                } else if !inner.paused() && inner.len() > MAX_PAYLOAD_SIZE {
                    drop(inner);
                    shared.borrow_mut().pause();
                    true
                } else {
                    inner.paused()
                }
            }
            None => false,
        }
    }
}

struct Inner {
    len: usize,
    eof: bool,
    paused: bool,
    task: Option<Task>,
    items: VecDeque<Bytes>,
}

impl Inner {

    fn new(eof: bool) -> Self {
        Inner {
            len: 0,
            eof: eof,
            paused: false,
            task: None,
            items: VecDeque::new(),
        }
    }

    fn paused(&self) -> bool {
        self.paused
    }

    fn pause(&mut self) {
        self.paused = true;
    }

    fn resume(&mut self) {
        self.paused = false;
    }

    fn feed_eof(&mut self) {
        self.eof = true;
        if let Some(task) = self.task.take() {
            task.notify()
        }
    }

    fn feed_data(&mut self, data: Bytes) {
        self.len += data.len();
        self.items.push_back(data);
        if let Some(task) = self.task.take() {
            task.notify()
        }
    }

    fn eof(&self) -> bool {
        self.eof
    }

    fn len(&self) -> usize {
        self.len
    }

    fn readany(&mut self) -> Async<Option<Bytes>> {
        if let Some(data) = self.items.pop_front() {
            self.len -= data.len();
            Async::Ready(Some(data))
        } else if self.eof {
            Async::Ready(None)
        } else {
            self.task = Some(current_task());
            Async::NotReady
        }
    }

    pub fn unread_data(&mut self, data: Bytes) {
        self.len += data.len();
        self.items.push_front(data)
    }
}
