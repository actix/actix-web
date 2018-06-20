use bytes::{BufMut, BytesMut};
use futures::Poll;
use std::io;

use super::IoStream;

const LW_BUFFER_SIZE: usize = 4096;
const HW_BUFFER_SIZE: usize = 32_768;

pub fn read_from_io<T: IoStream>(
    io: &mut T, buf: &mut BytesMut,
) -> Poll<usize, io::Error> {
    if buf.remaining_mut() < LW_BUFFER_SIZE {
        buf.reserve(HW_BUFFER_SIZE);
    }
    io.read_buf(buf)
}
