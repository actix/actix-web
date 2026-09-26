use std::{cell::Cell, pin::pin};

use actix_service::fn_service;
use tokio::io::ReadBuf;

use super::*;
use crate::{
    config::ServiceConfigBuilder,
    h1::{ExpectHandler, UpgradeHandler},
    test::TestBuffer,
};

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

async fn check_buffer_retention(body_len: usize, threshold: usize, partial: bool, release: bool) {
    let mut io = TestBuffer::new("GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let allowance = Rc::new(Cell::new(if partial {
        body_len - 1024
    } else {
        usize::MAX
    }));
    let services = HttpFlow::new(
        fn_service(move |_: Request| async move {
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
        ServiceConfigBuilder::new()
            .h1_write_buffer_size(threshold)
            .build(),
        None,
        OnConnectData::default(),
    );
    let mut dispatcher = pin!(dispatcher);
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());

    assert!(dispatcher.as_mut().poll(&mut cx).is_pending());

    if partial {
        let DispatcherState::Normal { inner } = &dispatcher.inner else {
            panic!()
        };
        assert!(!inner.write_buf.is_empty());
        assert_eq!(io.write_buf_slice().len(), body_len - 1024);

        allowance.set(usize::MAX);
        assert!(dispatcher.as_mut().poll(&mut cx).is_pending());
    }

    // Reclaim the consumed prefix so a small tail cannot hide a large allocation.
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
    check_buffer_retention(256 * 1024, 1024 * 1024, false, false).await;
}
