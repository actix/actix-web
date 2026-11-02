//! Direct file transfer for plain macOS and Linux TCP sockets.

use std::{
    any::Any,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use super::Codec;
use crate::body::MessageBody;

/// Returns `None` when the transport or body needs the byte-stream path.
pub(super) fn poll_sendfile<T: Any, B: MessageBody>(
    io: &T,
    body: Pin<&mut B>,
    codec: &mut Codec,
    cx: &mut Context<'_>,
) -> Option<Poll<io::Result<()>>> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use tokio::{io::Interest, net::TcpStream};

        let socket = (io as &dyn Any).downcast_ref::<TcpStream>()?;
        let mut region = body.file_region()?;
        let remaining = codec.file_remaining()?;

        if remaining == 0 || remaining != region.remaining() {
            return None;
        }

        let offset = i64::try_from(region.offset()).ok()?;
        let count = remaining.min(1024 * 1024) as i64;

        // try_io clears Tokio's readiness on EAGAIN. Poll again after that to register a wakeup.
        loop {
            match socket.poll_write_ready(cx) {
                Poll::Pending => return Some(Poll::Pending),
                Poll::Ready(Err(err)) => return Some(Poll::Ready(Err(err))),
                Poll::Ready(Ok(())) => {}
            }

            let mut sent = 0;
            let result = socket.try_io(Interest::WRITABLE, || {
                let (transferred, result) = transfer(region.file(), socket, offset, count);
                sent = transferred;
                tracing::trace!(sent, offset, error = ?result.as_ref().err(), "sendfile syscall");

                result
            });

            // macOS reports progress even when the syscall returns EAGAIN or EINTR.
            region.advance(sent);
            codec.advance_file(sent);

            match result {
                Ok(()) if sent == 0 => {
                    return Some(Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into())));
                }
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::WouldBlock && sent == 0 => continue,
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(err)
                    if sent == 0
                        && (matches!(err.raw_os_error(), Some(libc::ENOTSUP | libc::ENOSYS))
                            || (cfg!(target_os = "linux")
                                && err.raw_os_error() == Some(libc::EINVAL))) =>
                {
                    return None
                }
                Err(err) => return Some(Poll::Ready(Err(err))),
            }

            if region.remaining() == 0 {
                return Some(Poll::Ready(Ok(())));
            }

            // Bound work per poll so one large response cannot monopolize the worker.
            cx.waker().wake_by_ref();
            return Some(Poll::Pending);
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (io, body, codec, cx);
        None
    }
}

#[cfg(target_os = "macos")]
fn transfer(
    file: &std::fs::File,
    socket: &tokio::net::TcpStream,
    offset: i64,
    count: i64,
) -> (u64, io::Result<()>) {
    use std::os::fd::AsRawFd as _;

    let mut length = count;
    // SAFETY: both descriptors remain open for this call; length is a valid output pointer.
    // No header/trailer buffers are supplied. The socket is nonblocking.
    let result = unsafe {
        libc::sendfile(
            file.as_raw_fd(),
            socket.as_raw_fd(),
            offset,
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    let error = (result != 0).then(io::Error::last_os_error);
    // Only success, EAGAIN, and EINTR guarantee a transferred-byte count. Other errors can leave
    // length unchanged, so it would still contain the request size.
    let sent = if error.as_ref().is_none_or(|err| {
        matches!(
            err.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
        )
    }) {
        length as u64
    } else {
        0
    };

    (sent, error.map_or(Ok(()), Err))
}

#[cfg(target_os = "linux")]
fn transfer(
    file: &std::fs::File,
    socket: &tokio::net::TcpStream,
    mut offset: i64,
    count: i64,
) -> (u64, io::Result<()>) {
    use std::os::fd::AsRawFd as _;

    // SAFETY: both descriptors remain open for this call and offset is a valid output pointer.
    // The explicit offset leaves the file's seek position unchanged. The 64-bit interface also
    // supports large files on 32-bit Linux. count is bounded to 1 MiB by the caller.
    let result = unsafe {
        libc::sendfile64(
            socket.as_raw_fd(),
            file.as_raw_fd(),
            &mut offset,
            count as usize,
        )
    };

    if result < 0 {
        // Linux reports partial success as a positive return value, rather than an error with a
        // separate byte count. EAGAIN and EINTR therefore advance neither cursor.
        (0, Err(io::Error::last_os_error()))
    } else {
        (result as u64, Ok(()))
    }
}
