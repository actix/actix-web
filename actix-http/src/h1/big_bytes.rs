use std::collections::VecDeque;

use bytes::{Buf, BufMut, Bytes, BytesMut};

pub(crate) struct BigBytes {
    buffer: BytesMut,
    frozen: VecDeque<Bytes>,
    frozen_len: usize,
}

impl BigBytes {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: BytesMut::with_capacity(capacity),
            frozen: VecDeque::default(),
            frozen_len: 0,
        }
    }

    // Clear the internal queue and buffer, resetting length to zero
    pub(super) fn clear(&mut self) {
        std::mem::take(&mut self.frozen);
        self.frozen_len = 0;
        self.buffer.clear();
    }

    // Return a mutable reference to the underlying buffer. This should only be used when dealing
    // with small allocations (e.g. writing headers)
    pub(super) fn buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.buffer
    }

    pub(super) fn total_len(&mut self) -> usize {
        self.frozen_len + self.buffer.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.frozen_len == 0 && self.buffer.is_empty()
    }

    // Add the `bytes` to the internal structure. If `bytes` exceeds 64KB, it is pushed into a
    // queue, otherwise, it is added to a buffer.
    pub(super) fn put_bytes(&mut self, bytes: Bytes) {
        if !self.buffer.is_empty() {
            let current = self.buffer.split().freeze();
            self.frozen_len += current.len();
            self.frozen.push_back(current);
        }

        if !bytes.is_empty() {
            self.frozen_len += bytes.len();
            self.frozen.push_back(bytes);
        }
    }

    // Returns a slice of the frontmost buffer
    pub(super) fn front_slice(&self) -> &[u8] {
        if let Some(front) = self.frozen.front() {
            front
        } else {
            &self.buffer
        }
    }

    // Advances the first buffer by `count` bytes. If the first buffer is advanced to completion,
    // it is popped from the queue
    pub(super) fn advance(&mut self, count: usize) {
        if let Some(front) = self.frozen.front_mut() {
            front.advance(count);

            if front.is_empty() {
                self.frozen.pop_front();
            }

            self.frozen_len -= count;
        } else {
            self.buffer.advance(count);
        }
    }

    // Drain the BibBytes, writing everything into the provided BytesMut
    pub(super) fn write_to(&mut self, dst: &mut BytesMut) {
        dst.reserve(self.total_len());

        for buf in &self.frozen {
            dst.put_slice(buf);
        }

        dst.put_slice(&self.buffer.split());

        self.frozen_len = 0;

        std::mem::take(&mut self.frozen);
    }
}
