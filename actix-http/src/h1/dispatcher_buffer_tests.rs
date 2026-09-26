use std::{cell::Cell, pin::pin};

use actix_service::fn_service;
use tokio::io::ReadBuf;

use super::*;
use crate::{
    config::ServiceConfigBuilder,
    h1::{ExpectHandler, UpgradeHandler},
    test::TestBuffer,
};

/// Stops accepting writes after a shared byte budget is exhausted.
///
/// The test can restore the budget and poll the dispatcher again, as if the socket became writable.
struct LimitedWriter {
    io: TestBuffer,
    allowance: Rc<Cell<usize>>,
}

impl AsyncRead for LimitedWriter {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for LimitedWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let len = buf.len().min(self.allowance.get());

        if len == 0 {
            // The test polls manually after restoring the budget, so no wakeup is needed.
            return Poll::Pending;
        }

        let written = ready!(Pin::new(&mut self.io).poll_write(cx, &buf[..len]))?;
        self.allowance.set(self.allowance.get() - written);
        Poll::Ready(Ok(written))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

/// Checks buffer retention and complete responses on a persistent HTTP/1 connection.
///
/// - `body_len`: Response body length in bytes, sent as one chunk for each request.
/// - `threshold`: Configured HTTP/1 write-buffer watermark in bytes. The retention limit is
///   the larger of this value and `WRITE_BUFFER_RETENTION_LIMIT`.
/// - `partial`: If true, block writes after `body_len - 1024` bytes, including headers,
///   then resume them. Requires `body_len > 1024`.
/// - `release`: If true, expect the drained buffer to retain at most `HW_BUFFER_SIZE` bytes.
///   Otherwise, expect sufficient capacity for the body and reuse of the buffer on the next request.
async fn check_buffer_retention(body_len: usize, threshold: usize, partial: bool, release: bool) {
    let mut io = TestBuffer::new("GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    // The budget includes response headers. Leave a small unwritten tail so
    // advancing past the written bytes hides most of the original buffer capacity.
    let allowance = Rc::new(Cell::new(if partial {
        body_len - 1024
    } else {
        usize::MAX
    }));
    let services = HttpFlow::new(
        fn_service(move |_: Request| async move {
            // A single body chunk forces the output buffer to hold the large response.
            Ok::<_, Error>(Response::ok().set_body(bytes::Bytes::from(vec![b'x'; body_len])))
        }),
        ExpectHandler,
        None::<UpgradeHandler>,
    );
    let dispatcher = Dispatcher::new(
        LimitedWriter {
            io: io.clone(),
            allowance: allowance.clone(),
        },
        services,
        // This sets the body-polling watermark. The retention limit is the larger
        // of this value and WRITE_BUFFER_RETENTION_LIMIT.
        ServiceConfigBuilder::new()
            .h1_write_buffer_size(threshold)
            .build(),
        None,
        OnConnectData::default(),
    );
    let mut dispatcher = pin!(dispatcher);
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());

    // Pending is expected even after a complete response: the keep-alive
    // connection stays open and waits for another request.
    assert!(dispatcher.as_mut().poll(&mut cx).is_pending());

    if partial {
        let DispatcherState::Normal { inner } = &dispatcher.inner else {
            panic!()
        };
        assert!(!inner.write_buf.is_empty());
        assert_eq!(io.write_buf_slice().len(), body_len - 1024);

        // Let the dispatcher finish the same response after the simulated blocked write.
        allowance.set(usize::MAX);
        assert!(dispatcher.as_mut().poll(&mut cx).is_pending());
    }

    // try_reclaim does not allocate. If the old backing allocation survived,
    // reclaiming its consumed prefix exposes the large capacity to the assertion below.
    // Checking only the visible tail could otherwise let the partial-write regression pass.
    if release {
        let DispatcherStateProj::Normal { inner } = dispatcher.as_mut().project().inner.project()
        else {
            panic!()
        };
        assert!(inner.project().write_buf.try_reclaim(HW_BUFFER_SIZE));
    }

    let DispatcherState::Normal { inner } = &dispatcher.inner else {
        panic!()
    };
    assert!(inner.write_buf.is_empty());

    if release {
        assert!(
            inner.write_buf.capacity() <= HW_BUFFER_SIZE,
            "drained buffer retained {} bytes",
            inner.write_buf.capacity()
        );
    } else {
        assert!(inner.write_buf.capacity() >= body_len);
    }

    let pointer = inner.write_buf.as_ptr();
    let response = io.take_write_buf();
    // Compare the complete body separately from the generated HTTP headers.
    // A smaller retained buffer must not come at the cost of missing response bytes.
    let body_start = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    assert_eq!(&response[body_start..], vec![b'x'; body_len]);

    // The connection must remain usable, including after a partial write.
    io.extend_read_buf("GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert!(dispatcher.as_mut().poll(&mut cx).is_pending());
    let second_response = io.take_write_buf();
    let second_body_start = second_response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    assert_eq!(
        &second_response[second_body_start..],
        &response[body_start..]
    );

    if !release {
        // Check reuse across requests, not just sufficient capacity after each flush.
        let DispatcherState::Normal { inner } = &dispatcher.inner else {
            panic!()
        };
        assert_eq!(
            inner.write_buf.as_ptr(),
            pointer,
            "normal buffers should be reused"
        );
    }
}

#[actix_rt::test]
async fn oversized_write_buffer_released_after_flush() {
    check_buffer_retention(1024 * 1024, 32 * 1024, false, true).await;
}

#[actix_rt::test]
async fn oversized_write_buffer_released_after_partial_write() {
    check_buffer_retention(1024 * 1024, 32 * 1024, true, true).await;
}

#[actix_rt::test]
async fn normal_write_buffer_reused_after_flush() {
    check_buffer_retention(1024, 32 * 1024, false, false).await;
}

#[actix_rt::test]
async fn configured_write_buffer_reused_after_flush() {
    // A custom watermark above 128 KiB also raises the retention limit.
    check_buffer_retention(256 * 1024, 1024 * 1024, false, false).await;
}

#[actix_rt::test]
async fn small_write_watermark_preserves_normal_buffer() {
    // A small watermark must not lower the 128 KiB retention limit.
    check_buffer_retention(64 * 1024, 8 * 1024, false, false).await;
}

#[actix_rt::test]
async fn buffer_above_configured_write_watermark_released() {
    // A larger watermark sets the limit directly; it is not multiplied by four.
    check_buffer_retention(512 * 1024, 256 * 1024, false, true).await;
}
