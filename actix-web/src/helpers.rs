use std::io;

use bytes::BufMut;

/// An `io::Write`r that only requires mutable reference and assumes that there is space available
/// in the buffer for every write operation or that it can be extended implicitly (like
/// `bytes::BytesMut`, for example).
///
/// This is slightly faster (~10%) than `bytes::buf::Writer` in such cases because it does not
/// perform a remaining length check before writing.
pub(crate) struct MutWriter<'a, B>(pub(crate) &'a mut B);

impl<'a, B> io::Write for MutWriter<'a, B>
where
    B: BufMut,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.put_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
