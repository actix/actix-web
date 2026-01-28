use std::collections::VecDeque;

use bytes::{Buf, BufMut, Bytes, BytesMut};

// 64KB max capacity (arbitrarily chosen)
const MAX_CAPACITY: usize = 1024 * 64;

pub struct BigBytes {
    buffer: BytesMut,
    frozen: VecDeque<Bytes>,
    frozen_len: usize,
}

impl BigBytes {
    /// Initialize a new BigBytes with the internal buffer set to `capacity` capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: BytesMut::with_capacity(capacity),
            frozen: VecDeque::default(),
            frozen_len: 0,
        }
    }

    /// Clear the internal queue and buffer, resetting length to zero
    ///
    /// if the internal buffer capacity exceeds 64KB or new_capacity, whichever is greater, it will
    /// be freed and a new buffer of capacity `new_capacity` will be allocated
    pub fn clear(&mut self, new_capacity: usize) {
        std::mem::take(&mut self.frozen);
        self.frozen_len = 0;
        self.buffer.clear();

        if self.buffer.capacity() > new_capacity.max(MAX_CAPACITY) {
            self.buffer = BytesMut::with_capacity(new_capacity);
        }
    }

    /// Return a mutable reference to the underlying buffer. This should only be used when dealing
    /// with small allocations (e.g. writing headers)
    pub fn buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.buffer
    }

    /// Return the total length of the bytes stored in BigBytes
    pub fn total_len(&mut self) -> usize {
        self.frozen_len + self.buffer.len()
    }

    /// Return whether there are no bytes present in the BigBytes
    pub fn is_empty(&self) -> bool {
        self.frozen_len == 0 && self.buffer.is_empty()
    }

    /// Add the `bytes` to the internal structure. If `bytes` exceeds 64KB, it is pushed into a
    /// queue, otherwise, it is added to a buffer.
    pub fn put_bytes(&mut self, bytes: Bytes) {
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

    /// Returns a slice of the frontmost buffer
    ///
    /// While there are bytes present in BigBytes, front_slice is guaranteed not to return an empty
    /// slice.
    pub fn front_slice(&self) -> &[u8] {
        if let Some(front) = self.frozen.front() {
            front
        } else {
            &self.buffer
        }
    }

    /// Advances the first buffer by `count` bytes. If the first buffer is advanced to completion,
    /// it is popped from the queue
    pub fn advance(&mut self, count: usize) {
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

    /// Pops the front Bytes from the BigBytes, or splits and freezes the internal buffer if no
    /// Bytes are present.
    pub fn pop_front(&mut self) -> Option<Bytes> {
        if let Some(front) = self.frozen.pop_front() {
            self.frozen_len -= front.len();
            Some(front)
        } else if !self.buffer.is_empty() {
            Some(self.buffer.split().freeze())
        } else {
            None
        }
    }

    /// Drain the BigBytes, writing everything into the provided BytesMut
    pub fn write_to(&mut self, dst: &mut BytesMut) {
        dst.reserve(self.total_len());

        for buf in &self.frozen {
            dst.put_slice(buf);
        }

        dst.put_slice(&self.buffer.split());

        self.frozen_len = 0;

        std::mem::take(&mut self.frozen);
    }
}
