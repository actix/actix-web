use bytes::{BufMut, BytesMut};
use futures::{Async, Poll};
use std::io;

use super::IoStream;

const LW_BUFFER_SIZE: usize = 4096;
const HW_BUFFER_SIZE: usize = 32_768;

pub fn read_from_io<T: IoStream>(
    io: &mut T, buf: &mut BytesMut,
) -> Poll<usize, io::Error> {
    unsafe {
        if buf.remaining_mut() < LW_BUFFER_SIZE {
            buf.reserve(HW_BUFFER_SIZE);
        }
        match io.read(buf.bytes_mut()) {
            Ok(n) => {
                buf.advance_mut(n);
                Ok(Async::Ready(n))
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::WouldBlock {
                    Ok(Async::NotReady)
                } else {
                    Err(e)
                }
            }
        }
    }
}
